//! Offline mixing keeps one bounded PCM block and one decoder active, independent of timeline length.

use std::{
    io::{Read, Seek, SeekFrom, Write},
    process::{Command, Stdio},
};

use super::*;
use crate::media_process::MediaChild;

const SAMPLE_RATE: u64 = AUDIO_SAMPLE_RATE as u64;
const CHANNELS: usize = crate::audio::CHANNELS;
const SAMPLE_BYTES: usize = std::mem::size_of::<f32>();
const FRAME_BYTES: usize = CHANNELS * SAMPLE_BYTES;
const BLOCK_FRAMES: usize = crate::audio::MAX_BLOCK_FRAMES;
const MIX_BYTE_LIMIT: u64 = 16 * 1024 * 1024 * 1024;

/// Scratch duration is admitted before video encoding so a long project cannot fail this check at the end.
pub(super) fn byte_length(fps: crate::FrameRate, video_frames: u64) -> Result<u64, String> {
    sample_count(fps, video_frames)?
        .checked_mul(FRAME_BYTES as u64)
        .filter(|bytes| *bytes <= MIX_BYTE_LIMIT)
        .ok_or_else(|| "The audio export exceeds its scratch storage limit.".into())
}

/// Disk-backed scratch storage preserves silence without allocating an in-memory track per source.
pub(super) fn mix(
    request: &ExportRequest,
    video_frames: u64,
    scratch: &Path,
    cancellation: &Cancellation,
) -> Result<PathBuf, String> {
    let frames = sample_count(request.project.export.fps, video_frames)?;
    let range = request.frame_range()?;
    let [origin, _, admitted_frames] = range
        .audio_bounds(SAMPLE_RATE as u32)
        .map_err(|error| error.to_string())?;
    if admitted_frames != frames {
        return Err("Export video and audio range disagree.".into());
    }
    let bytes = byte_length(request.project.export.fps, video_frames)?;
    let path = scratch.join("mixed-audio.f32");
    let mut mixed = File::options()
        .create_new(true)
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|error| error.to_string())?;
    mixed.set_len(bytes).map_err(|error| error.to_string())?;
    let mut decoded = vec![[0.0; CHANNELS]; BLOCK_FRAMES];
    let mut retained = vec![[0.0; CHANNELS]; BLOCK_FRAMES];
    let mut bytes = vec![0; BLOCK_FRAMES * FRAME_BYTES];
    for clip in &request.project.audio {
        cancellation.check()?;
        if request.project.audio_muted(clip) || clip.gain == 0.0 {
            continue;
        }
        let map = clip.time_map().map_err(|error| error.to_string())?;
        let span = map.span();
        let Some([start, end]) = range
            .audio_window(span, AUDIO_SAMPLE_RATE)
            .map_err(|error| error.to_string())?
        else {
            continue;
        };
        let source = request
            .project
            .sources
            .iter()
            .find(|source| source.id == clip.source)
            .ok_or("Audio source is missing.")?;
        let source_path = crate::audio::source_path(source, &request.source_base);
        let mut decoder = crate::audio::ClipDecoder::open(
            &source_path,
            clip,
            map,
            start,
            SAMPLE_RATE as u32,
            cancellation,
        )?;
        let mut position = start;
        while position < end {
            cancellation.check()?;
            let block = (end - position).min(BLOCK_FRAMES as u64) as usize;
            let length = block * FRAME_BYTES;
            decoder.read(&mut decoded[..block], position, cancellation)?;
            mixed
                .seek(SeekFrom::Start((position - origin) * FRAME_BYTES as u64))
                .map_err(|error| error.to_string())?;
            mixed
                .read_exact(&mut bytes[..length])
                .map_err(|error| error.to_string())?;
            for (frame, bytes) in retained[..block]
                .iter_mut()
                .zip(bytes[..length].chunks_exact(FRAME_BYTES))
            {
                for (sample, bytes) in frame.iter_mut().zip(bytes.chunks_exact(SAMPLE_BYTES)) {
                    *sample = f32::from_le_bytes(bytes.try_into().expect("one PCM sample"));
                }
            }
            crate::audio::mix_into(
                clip,
                span,
                position,
                SAMPLE_RATE as u32,
                &decoded[..block],
                &mut retained[..block],
            )?;
            for (frame, bytes) in retained[..block]
                .iter()
                .zip(bytes[..length].chunks_exact_mut(FRAME_BYTES))
            {
                for (sample, bytes) in frame.iter().zip(bytes.chunks_exact_mut(SAMPLE_BYTES)) {
                    bytes.copy_from_slice(&sample.to_le_bytes());
                }
            }
            mixed
                .seek(SeekFrom::Start((position - origin) * FRAME_BYTES as u64))
                .map_err(|error| error.to_string())?;
            mixed
                .write_all(&bytes[..length])
                .map_err(|error| error.to_string())?;
            position += block as u64;
        }
        if end == sample_frame(span.end)? {
            decoder.finish(cancellation)?;
        } else {
            // A range prefix intentionally stops before the decoder's full retained clip extent.
            // Dropping on this worker cancels and reaps its child without decoding the unused tail.
            drop(decoder);
        }
    }
    mixed.sync_all().map_err(|error| error.to_string())?;
    Ok(path)
}

/// AAC muxing retains every encoded video packet; the PCM mix already owns its exact sample extent.
pub(super) fn mux(
    video: &Path,
    audio: &Path,
    output: &Path,
    cancellation: &Cancellation,
) -> Result<(), String> {
    let executable = crate::video::find_ffmpeg()?;
    let mut command = Command::new(executable);
    command
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-y",
            "-protocol_whitelist",
            "file,pipe",
            "-i",
        ])
        .arg(video)
        .args([
            "-f",
            "f32le",
            "-ar",
            &SAMPLE_RATE.to_string(),
            "-ac",
            &CHANNELS.to_string(),
            "-i",
        ])
        .arg(audio)
        .args([
            "-map", "0:v:0", "-map", "1:a:0", "-c:v", "copy", "-c:a", "aac", "-b:a", "192k",
        ])
        .args(["-movflags", "+faststart"])
        .arg(output)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    let relocation_bytes = std::fs::metadata(video)
        .map_err(|error| error.to_string())?
        .len();
    MediaChild::start_output(command, cancellation.clone(), output, relocation_bytes)?.finish()
}

/// Ceiling assigns every half-open timeline boundary to the same output sample grid.
fn sample_frame(at: MediaTime) -> Result<u64, String> {
    crate::audio::sample_frame(at, SAMPLE_RATE as u32)
}

/// The encoded video ends on a rational frame boundary; rounding through microseconds loses samples.
fn sample_count(fps: crate::FrameRate, frames: u64) -> Result<u64, String> {
    fps.validate().map_err(|error| error.to_string())?;
    let numerator = u128::from(frames) * u128::from(fps.denominator) * u128::from(SAMPLE_RATE);
    u64::try_from(numerator.div_ceil(u128::from(fps.numerator)))
        .map_err(|_| "The export audio sample extent overflows.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsampled_audio_prefix_never_opens_a_decoder_and_leaves_silent_output() {
        use crate::{
            Id, PlaybackRate, Span,
            model::{AudioClip, AudioRole, MediaKind, MediaSource},
        };
        let root = tempfile::tempdir().unwrap();
        let mut project = Project::new("silent prefix");
        project.export.fps = crate::FrameRate {
            numerator: 24,
            denominator: 1,
        };
        project.export.range = Some(Span::new(MediaTime(33_333), MediaTime(100_000)).unwrap());
        project.sources.push(MediaSource {
            id: Id(1),
            name: "missing source".into(),
            path: root.path().join("missing.wav"),
            kind: MediaKind::Audio,
            duration: MediaTime(1_000_000),
            size: [0, 0],
            frame_rate: None,
            sha256: None,
        });
        let mut audio = AudioClip {
            id: Id(2),
            source: Id(1),
            linked_clip: None,
            source_range: Span::new(MediaTime::ZERO, MediaTime(5_000)).unwrap(),
            at: MediaTime(35_000),
            rate: PlaybackRate::default(),
            speed_ramp: None,
            role: AudioRole::Music,
            gain: 1.0,
            fade_in: MediaTime::ZERO,
            fade_out: MediaTime::ZERO,
            muted: false,
        };
        project.audio.push(audio.clone());
        audio.id = Id(3);
        audio.at = MediaTime::ZERO;
        audio.source_range.end = MediaTime(1_000_000);
        audio.muted = true;
        project.audio.push(audio);
        project.validate().unwrap();
        let request = ExportRequest {
            project: Arc::new(project),
            source_base: root.path().into(),
            destination: Destination::Directory(root.path().into()),
        };
        let frames = request.frame_range().unwrap().frames();
        let path = mix(&request, frames, root.path(), &Cancellation::default()).unwrap();
        let bytes = std::fs::read(path).unwrap();
        assert_eq!(
            bytes.len() as u64,
            byte_length(request.project.export.fps, frames).unwrap()
        );
        assert!(bytes.into_iter().all(|byte| byte == 0));
    }

    #[test]
    fn range_ending_inside_an_audio_clip_mixes_only_that_prefix_and_preserves_its_fade() {
        use crate::{
            Id, PlaybackRate, Span,
            model::{AudioClip, AudioRole, MediaKind, MediaSource},
        };
        if crate::video::find_ffmpeg().is_err() {
            eprintln!("missing-fixture: FFmpeg for export range PCM decoding");
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("constant.wav");
        let sample_rate = SAMPLE_RATE as u32;
        let block = CHANNELS as u16 * std::mem::size_of::<i16>() as u16;
        let bytes = sample_rate * 2 * u32::from(block);
        let mut wave = Vec::with_capacity(bytes as usize + 44);
        wave.extend_from_slice(b"RIFF");
        wave.extend_from_slice(&(bytes + 36).to_le_bytes());
        wave.extend_from_slice(b"WAVEfmt ");
        wave.extend_from_slice(&16_u32.to_le_bytes());
        wave.extend_from_slice(&1_u16.to_le_bytes());
        wave.extend_from_slice(&(CHANNELS as u16).to_le_bytes());
        wave.extend_from_slice(&sample_rate.to_le_bytes());
        wave.extend_from_slice(&(sample_rate * u32::from(block)).to_le_bytes());
        wave.extend_from_slice(&block.to_le_bytes());
        wave.extend_from_slice(&16_u16.to_le_bytes());
        wave.extend_from_slice(b"data");
        wave.extend_from_slice(&bytes.to_le_bytes());
        for _ in 0..sample_rate as usize * 2 * CHANNELS {
            wave.extend_from_slice(&i16::MAX.to_le_bytes());
        }
        std::fs::write(&path, wave).unwrap();
        let mut project = Project::new("audio prefix");
        project.sources.push(MediaSource {
            id: Id(1),
            name: "constant".into(),
            path,
            kind: MediaKind::Audio,
            duration: MediaTime(2_000_000),
            size: [0, 0],
            frame_rate: None,
            sha256: None,
        });
        project.audio.push(AudioClip {
            id: Id(2),
            source: Id(1),
            linked_clip: None,
            source_range: Span::new(MediaTime::ZERO, MediaTime(2_000_000)).unwrap(),
            at: MediaTime::ZERO,
            rate: PlaybackRate::default(),
            speed_ramp: None,
            role: AudioRole::Music,
            gain: 1.0,
            fade_in: MediaTime(1_000_000),
            fade_out: MediaTime::ZERO,
            muted: false,
        });
        project.export.range = Some(Span::new(MediaTime(500_000), MediaTime(1_000_000)).unwrap());
        let request = ExportRequest {
            project: Arc::new(project),
            source_base: root.path().into(),
            destination: Destination::Directory(root.path().into()),
        };
        let range = request.frame_range().unwrap();
        let output = mix(
            &request,
            range.frames(),
            root.path(),
            &Cancellation::default(),
        )
        .unwrap();
        let mixed = std::fs::read(output).unwrap();
        assert_eq!(
            mixed.len() as u64,
            range.audio_bounds(sample_rate).unwrap()[2] * FRAME_BYTES as u64
        );
        let first = f32::from_le_bytes(mixed[..SAMPLE_BYTES].try_into().unwrap());
        assert!((first - 0.5).abs() < 0.0001);
    }

    #[test]
    fn clipped_mix_retains_global_decoder_and_fade_time_and_rebases_only_the_output() {
        use crate::{
            Id, PlaybackRate, Span,
            model::{AudioClip, AudioRole},
        };
        let span = Span::new(MediaTime(2_000_000), MediaTime(5_000_000)).unwrap();
        let audio = AudioClip {
            id: Id(1),
            source: Id(2),
            linked_clip: None,
            source_range: Span::new(MediaTime::ZERO, MediaTime(3_000_000)).unwrap(),
            at: span.start,
            rate: PlaybackRate::default(),
            speed_ramp: None,
            role: AudioRole::Music,
            gain: 1.0,
            fade_in: MediaTime(2_000_000),
            fade_out: MediaTime::ZERO,
            muted: false,
        };
        let origin = sample_frame(MediaTime(3_000_000)).unwrap();
        let end = sample_frame(MediaTime(4_000_000)).unwrap();
        let range = crate::export::range::FrameRange::new(
            crate::FrameRate::default(),
            span.end,
            Some(Span::new(MediaTime(3_000_000), MediaTime(4_000_000)).unwrap()),
        )
        .unwrap();
        assert_eq!(
            range.audio_window(span, AUDIO_SAMPLE_RATE).unwrap(),
            Some([origin, end])
        );
        assert!(
            range
                .audio_window(
                    Span::new(MediaTime::ZERO, MediaTime(1_000_000)).unwrap(),
                    AUDIO_SAMPLE_RATE,
                )
                .unwrap()
                .is_none()
        );
        let mut output = [[0.0; CHANNELS]; 1];
        crate::audio::mix_into(
            &audio,
            span,
            origin,
            SAMPLE_RATE as u32,
            &[[1.0; CHANNELS]],
            &mut output,
        )
        .unwrap();
        assert!((output[0][0] - 0.5).abs() < 0.00001);
    }

    #[test]
    fn final_pcm_extent_uses_the_video_rational_clock_without_microsecond_loss() {
        let fps = crate::FrameRate {
            numerator: 239,
            denominator: 1,
        };
        // This endpoint's fractional sample lies inside one microsecond of the preceding sample.
        let frames = 49;
        let exact = sample_count(fps, frames).unwrap();
        assert_eq!(exact, (frames * SAMPLE_RATE).div_ceil(239));
        assert_eq!(exact, 9842);
        assert_eq!(
            sample_frame(fps.frame_time(frames).unwrap()).unwrap(),
            exact - 1
        );
        for numerator in [24, 30, 60, 120, 240, 30_000, 60_000] {
            let fps = crate::FrameRate {
                numerator,
                denominator: if numerator > 240 { 1001 } else { 1 },
            };
            for frames in [1, 61, 100_000] {
                let expected =
                    (u128::from(frames) * u128::from(fps.denominator) * u128::from(SAMPLE_RATE))
                        .div_ceil(u128::from(fps.numerator));
                assert_eq!(u128::from(sample_count(fps, frames).unwrap()), expected);
            }
        }
    }
}
