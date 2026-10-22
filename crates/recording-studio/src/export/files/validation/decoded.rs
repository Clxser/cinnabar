//! Export publication requires the requested streams and a complete, error-free video decode.

use std::{
    io::{BufRead, BufReader, Read},
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};

use serde::Deserialize;

use crate::{
    FrameRate,
    export::{AUDIO_SAMPLE_RATE, Cancellation},
    media_process::MediaChild,
};

const MAX_METADATA_BYTES: u64 = 64 * 1024;
const MAX_PROGRESS_LINE: u64 = 256;
const AAC_PACKET_SAMPLES: f64 = 1024.0;

#[derive(Clone, Copy)]
struct Expected {
    size: [u32; 2],
    fps: FrameRate,
    frames: u64,
    audio: bool,
}

#[derive(Deserialize)]
struct Probe {
    streams: Vec<Stream>,
}

#[derive(Deserialize)]
struct Stream {
    codec_type: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    avg_frame_rate: Option<String>,
    duration: Option<String>,
    start_time: Option<String>,
    nb_frames: Option<String>,
    sample_rate: Option<String>,
    channels: Option<u32>,
}

/// Checks metadata and decodes every packet on the export worker before any final file is published.
pub(in crate::export) fn verify_media(
    path: &Path,
    size: [u32; 2],
    fps: FrameRate,
    frames: u64,
    audio: bool,
    cancellation: &Cancellation,
) -> Result<(), String> {
    let expected = Expected {
        size,
        fps,
        frames,
        audio,
    };
    expected.fps.validate().map_err(|error| error.to_string())?;
    if frames == 0 {
        return Err("The export has no output frames to verify.".into());
    }
    cancellation.check()?;
    let ffmpeg = crate::video::find_ffmpeg()?;
    let probe = crate::media::probe_executable(&ffmpeg)?;
    let mut command = Command::new(probe);
    command.args([
        "-v", "error", "-protocol_whitelist", "file,pipe", "-show_entries",
        "stream=codec_type,width,height,avg_frame_rate,duration,start_time,nb_frames,sample_rate,channels",
        "-of", "json",
    ]).arg(path).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child =
        MediaChild::start_timed(command, cancellation.clone(), Duration::from_secs(30))?;
    let mut bytes = Vec::new();
    child
        .stdout
        .as_mut()
        .ok_or("Export validation metadata is unavailable.")?
        .take(MAX_METADATA_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_METADATA_BYTES {
        return Err("Export validation metadata exceeds its byte budget.".into());
    }
    child.finish()?;
    validate_metadata(&bytes, expected)?;

    let mut command = Command::new(ffmpeg);
    command
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-xerror",
            "-err_detect",
            "explode",
            "-threads",
            "2",
            "-protocol_whitelist",
            "file,pipe",
            "-i",
        ])
        .arg(path)
        .args([
            "-map",
            "0:v:0",
            "-map",
            "0:a:0?",
            "-sn",
            "-dn",
            "-fps_mode",
            "passthrough",
            "-threads:v",
            "1",
            "-progress",
            "pipe:1",
            "-stats_period",
            "1",
            "-nostats",
            "-f",
            "null",
            "-",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = MediaChild::start(command, cancellation.clone())?;
    let stdout = child
        .stdout
        .take()
        .ok_or("Export verification progress is unavailable.")?;
    let mut input = BufReader::with_capacity(512, stdout);
    let mut progress = Progress::default();
    let mut line = Vec::with_capacity(MAX_PROGRESS_LINE as usize);
    loop {
        cancellation.check()?;
        line.clear();
        let _deadline = child.guard_io();
        let count = (&mut input)
            .take(MAX_PROGRESS_LINE + 1)
            .read_until(b'\n', &mut line)
            .map_err(|error| format!("Cannot read export verification: {error}"))?;
        if count == 0 {
            break;
        }
        if count as u64 > MAX_PROGRESS_LINE {
            return Err("Export verification returned an oversized progress record.".into());
        }
        progress.observe(&line, frames)?;
    }
    child.finish().map_err(|error| {
        format!("The exported video or audio did not decode completely: {error}")
    })?;
    progress.finish(frames)?;
    cancellation.check()
}

/// The metadata must describe the exact rendered extent, cadence, duration and requested audio.
fn validate_metadata(bytes: &[u8], expected: Expected) -> Result<(), String> {
    let probe: Probe = serde_json::from_slice(bytes)
        .map_err(|error| format!("Invalid export validation metadata: {error}"))?;
    let videos = probe
        .streams
        .iter()
        .filter(|stream| stream.codec_type.as_deref() == Some("video"))
        .collect::<Vec<_>>();
    let audios = probe
        .streams
        .iter()
        .filter(|stream| stream.codec_type.as_deref() == Some("audio"))
        .collect::<Vec<_>>();
    if videos.len() != 1
        || audios.len() != usize::from(expected.audio)
        || probe.streams.len() != videos.len() + audios.len()
    {
        return Err(
            "The encoded export does not contain its requested video and audio streams.".into(),
        );
    }
    let video = videos[0];
    let fps = video
        .avg_frame_rate
        .as_deref()
        .and_then(|value| value.split_once('/'))
        .and_then(|(n, d)| Some((n.parse::<u64>().ok()?, d.parse::<u64>().ok()?)));
    let valid_fps = fps.is_some_and(|(n, d)| {
        d != 0
            && u128::from(n) * u128::from(expected.fps.denominator)
                == u128::from(d) * u128::from(expected.fps.numerator)
    });
    if [video.width, video.height] != expected.size.map(Some) || !valid_fps {
        return Err(
            "The encoded export has a different resolution or frame rate than requested.".into(),
        );
    }
    if video
        .nb_frames
        .as_deref()
        .and_then(|value| value.parse::<u64>().ok())
        != Some(expected.frames)
    {
        return Err("The encoded export metadata has an unexpected video frame count.".into());
    }
    let duration = expected.frames as f64 * f64::from(expected.fps.denominator)
        / f64::from(expected.fps.numerator);
    check_timing(video, duration, 0.000_002, "video")?;
    if let Some(audio) = audios.first() {
        if audio
            .sample_rate
            .as_deref()
            .and_then(|rate| rate.parse::<u32>().ok())
            != Some(AUDIO_SAMPLE_RATE)
            || audio.channels != Some(crate::audio::CHANNELS as u32)
        {
            return Err(
                "The encoded audio has a different sample rate or channel count than requested."
                    .into(),
            );
        }
        let tolerance = AAC_PACKET_SAMPLES / f64::from(AUDIO_SAMPLE_RATE) + 0.000_002;
        check_timing(audio, duration, tolerance, "audio")?;
    }
    Ok(())
}

/// Container timestamps must remain finite and start at zero; codec padding receives a bounded allowance.
fn check_timing(stream: &Stream, expected: f64, tolerance: f64, name: &str) -> Result<(), String> {
    let number = |value: &Option<String>| {
        value
            .as_deref()
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| value.is_finite())
    };
    let valid = number(&stream.start_time).is_some_and(|start| start.abs() <= 0.000_002)
        && number(&stream.duration)
            .is_some_and(|duration| (duration - expected).abs() <= tolerance);
    if valid {
        Ok(())
    } else {
        Err(format!(
            "The encoded {name} timestamps do not match the exported duration."
        ))
    }
}

#[derive(Default)]
struct Progress {
    frames: u64,
    ended: bool,
}

impl Progress {
    /// Only monotonically decoded frame counts and an explicit final record establish completion.
    fn observe(&mut self, line: &[u8], expected: u64) -> Result<(), String> {
        let Some((key, value)) = std::str::from_utf8(line)
            .ok()
            .and_then(|line| line.trim().split_once('='))
        else {
            return Ok(());
        };
        match key {
            "frame" => {
                let frames = value
                    .trim()
                    .parse::<u64>()
                    .map_err(|_| "Invalid export verification frame count.")?;
                if frames < self.frames || frames > expected {
                    return Err("The decoded export has an unexpected video frame count.".into());
                }
                self.frames = frames;
            }
            "progress" if value == "end" => self.ended = true,
            _ => {}
        }
        Ok(())
    }

    /// Successful process exit alone cannot publish an empty or truncated decode.
    fn finish(&self, expected: u64) -> Result<(), String> {
        if self.ended && self.frames == expected {
            Ok(())
        } else {
            Err("The exported video ended before every requested frame could be verified.".into())
        }
    }
}

#[cfg(test)]
mod tests;
