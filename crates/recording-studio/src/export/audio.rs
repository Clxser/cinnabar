//! Offline mixing keeps one bounded PCM block and one decoder active, independent of timeline length.

use std::{
    io::{Read, Seek, SeekFrom, Write},
    process::{Command, Stdio},
};

use super::*;
use crate::{media_process::MediaChild, model::MediaKind, time::MICROS_PER_SECOND};

const SAMPLE_RATE: u64 = 48_000;
const CHANNELS: usize = 2;
const SAMPLE_BYTES: usize = std::mem::size_of::<f32>();
const FRAME_BYTES: usize = CHANNELS * SAMPLE_BYTES;
const BLOCK_FRAMES: usize = 16_384;
const MIX_BYTE_LIMIT: u64 = 16 * 1024 * 1024 * 1024;

/// Scratch duration is admitted before video encoding so a long project cannot fail this check at the end.
pub(super) fn byte_length(duration: MediaTime) -> Result<u64, String> {
    sample_frame(duration)?
        .checked_mul(FRAME_BYTES as u64)
        .filter(|bytes| *bytes <= MIX_BYTE_LIMIT)
        .ok_or_else(|| "The audio export exceeds its scratch storage limit.".into())
}

/// Disk-backed scratch storage preserves silence without allocating an in-memory track per source.
pub(super) fn mix(
    request: &ExportRequest,
    duration: MediaTime,
    scratch: &Path,
    cancellation: &Cancellation,
) -> Result<PathBuf, String> {
    let frames = sample_frame(duration)?;
    let bytes = byte_length(duration)?;
    let path = scratch.join("mixed-audio.f32");
    let mut mixed = File::options()
        .create_new(true)
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|error| error.to_string())?;
    mixed.set_len(bytes).map_err(|error| error.to_string())?;
    let executable = crate::video::find_ffmpeg()?;
    let mut decoded = vec![0; BLOCK_FRAMES * FRAME_BYTES];
    let mut retained = vec![0; BLOCK_FRAMES * FRAME_BYTES];
    for clip in &request.project.audio {
        cancellation.check()?;
        if clip.muted || clip.gain == 0.0 {
            continue;
        }
        let span = clip.span().map_err(|error| error.to_string())?;
        let start = sample_frame(span.start)?.min(frames);
        let end = sample_frame(span.end)?.min(frames);
        if end <= start {
            continue;
        }
        let source = request
            .project
            .sources
            .iter()
            .find(|source| source.id == clip.source)
            .ok_or("Audio source is missing.")?;
        let mut source_path = super::source_path(&request.source_base, &source.path);
        if source.kind == MediaKind::Replay {
            source_path = source_path.join(crate::capture::AUDIO_FILE);
        }
        if !fs::metadata(&source_path)
            .map_err(|error| format!("Cannot open exported audio: {error}"))?
            .is_file()
        {
            return Err("Exported audio must be a local regular file.".into());
        }
        let count = end - start;
        let mut command = Command::new(&executable);
        command
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-threads",
                "2",
                "-protocol_whitelist",
                "file,pipe",
                "-ss",
            ])
            .arg(seconds(clip.source_range.start))
            .arg("-t")
            .arg(seconds(clip.source_range.duration()))
            .arg("-i")
            .arg(source_path)
            .args([
                "-map",
                "0:a:0",
                "-vn",
                "-sn",
                "-dn",
                "-filter_threads",
                "1",
                "-af",
            ])
            .arg(filter(clip, count))
            .args([
                "-ac",
                &CHANNELS.to_string(),
                "-ar",
                &SAMPLE_RATE.to_string(),
                "-f",
                "f32le",
                "pipe:1",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        let mut decoder = MediaChild::start(command, cancellation.clone())?;
        let decode_result = (|| {
            let output = decoder
                .stdout
                .as_mut()
                .ok_or("Audio decoder output is unavailable.")?;
            let mut position = start;
            while position < end {
                cancellation.check()?;
                let block_frames = (end - position).min(BLOCK_FRAMES as u64) as usize;
                let length = block_frames * FRAME_BYTES;
                output
                    .read_exact(&mut decoded[..length])
                    .map_err(|error| format!("Audio decode ended early: {error}"))?;
                mixed
                    .seek(SeekFrom::Start(position * FRAME_BYTES as u64))
                    .map_err(|error| error.to_string())?;
                mixed
                    .read_exact(&mut retained[..length])
                    .map_err(|error| error.to_string())?;
                for frame in 0..block_frames {
                    let at = MediaTime(
                        ((u128::from(position) + frame as u128) * u128::from(MICROS_PER_SECOND)
                            / u128::from(SAMPLE_RATE)) as u64,
                    );
                    let gain = clip.gain_at(at) as f32;
                    for channel in 0..CHANNELS {
                        let offset = frame * FRAME_BYTES + channel * SAMPLE_BYTES;
                        let value = f32::from_le_bytes(
                            decoded[offset..offset + SAMPLE_BYTES]
                                .try_into()
                                .expect("one sample"),
                        );
                        let old = f32::from_le_bytes(
                            retained[offset..offset + SAMPLE_BYTES]
                                .try_into()
                                .expect("one sample"),
                        );
                        let sum = old + value * gain;
                        if !value.is_finite() || !sum.is_finite() {
                            return Err("Audio contains non-finite sample values.".into());
                        }
                        retained[offset..offset + SAMPLE_BYTES].copy_from_slice(&sum.to_le_bytes());
                    }
                }
                mixed
                    .seek(SeekFrom::Start(position * FRAME_BYTES as u64))
                    .map_err(|error| error.to_string())?;
                mixed
                    .write_all(&retained[..length])
                    .map_err(|error| error.to_string())?;
                position += block_frames as u64;
            }
            Ok::<_, String>(())
        })();
        decode_result?;
        decoder.finish()?;
    }
    mixed.sync_all().map_err(|error| error.to_string())?;
    Ok(path)
}

/// Pitch-preserving tempo is factored into [0.5, 2] stages to avoid atempo's sample-skipping range.
fn filter(clip: &crate::model::AudioClip, frames: u64) -> String {
    let mut stages = vec![
        format!("atrim=duration={}", seconds(clip.source_range.duration())),
        "asetpts=PTS-STARTPTS".into(),
    ];
    let mut rate = f64::from(clip.rate.numerator) / f64::from(clip.rate.denominator);
    while rate > 2.0 {
        stages.push("atempo=2".into());
        rate /= 2.0;
    }
    while rate < 0.5 {
        stages.push("atempo=0.5".into());
        rate *= 2.0;
    }
    if rate != 1.0 {
        stages.push(format!("atempo={rate:.12}"));
    }
    stages.push(format!("aresample={SAMPLE_RATE}"));
    stages.push(format!("apad=whole_len={frames}"));
    stages.push(format!("atrim=end_sample={frames}"));
    stages.join(",")
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
    MediaChild::start(command, cancellation.clone())?.finish()
}

/// Ceiling assigns every half-open timeline boundary to the same output sample grid.
fn sample_frame(at: MediaTime) -> Result<u64, String> {
    u64::try_from(
        (u128::from(at.0) * u128::from(SAMPLE_RATE)).div_ceil(u128::from(MICROS_PER_SECOND)),
    )
    .map_err(|_| "Audio sample position overflow.".into())
}

/// Filter values are generated numeric literals, so media filenames never enter filter syntax.
fn seconds(at: MediaTime) -> String {
    format!(
        "{}.{:06}",
        at.0 / MICROS_PER_SECOND,
        at.0 % MICROS_PER_SECOND
    )
}
