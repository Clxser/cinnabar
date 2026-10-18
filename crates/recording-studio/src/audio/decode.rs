//! FFmpeg decoding is worker-local; bounded phase-aligned overlap preserves pitch for authored ramps.

use super::{CHANNELS, MAX_BLOCK_FRAMES, sample_frame};
use crate::{
    MediaTime,
    export::Cancellation,
    media_process::MediaChild,
    model::AudioClip,
    retime::{TimeMap, pcm::PitchWarp},
};
use std::{
    io::{BufReader, Read},
    path::Path,
    process::{ChildStdout, Command, Stdio},
};

enum Samples {
    Uniform(BufReader<ChildStdout>),
    Ramped(PitchWarp<ChildStdout>),
}

pub struct ClipDecoder {
    child: MediaChild,
    samples: Samples,
    next: u64,
    end: u64,
    bytes: Vec<u8>,
}

impl ClipDecoder {
    /// Starts at the exact requested output sample; seeking never decodes the entire preceding clip.
    pub fn open(
        path: &Path,
        clip: &AudioClip,
        map: TimeMap,
        start: u64,
        rate: u32,
        cancellation: &Cancellation,
    ) -> Result<Self, String> {
        cancellation.check()?;
        if !std::fs::metadata(path)
            .map_err(|error| format!("Cannot open Studio audio: {error}"))?
            .is_file()
        {
            return Err("Studio audio must be a local regular file.".into());
        }
        let end = sample_frame(map.span().end, rate)?;
        if start >= end {
            return Err("The audio decoder starts beyond its retained clip.".into());
        }
        let (position, _) = map
            .sample_clock(start, rate)
            .map_err(|error| error.to_string())?;
        let origin = MediaTime(
            (position.floor() as u64).clamp(clip.source_range.start.0, clip.source_range.end.0 - 1),
        );
        let remaining = clip.source_range.end.saturating_sub(origin);
        let frames = end - start;
        let uniform = map.uniform_rate();
        let filter = if let Some((numerator, denominator)) = uniform {
            uniform_filter(
                numerator as f64 / denominator as f64,
                frames,
                rate,
                remaining,
            )
        } else {
            let source_frames = sample_frame(remaining, rate)?;
            let padding = PitchWarp::<ChildStdout>::padding_frames(rate)? as u64;
            let padded = source_frames
                .checked_add(padding)
                .ok_or("Audio padding overflows.")?;
            format!(
                "aresample={rate},atrim=end_sample={source_frames},asetpts=PTS-STARTPTS,apad=whole_len={padded},atrim=end_sample={padded}"
            )
        };
        let mut command = Command::new(crate::video::find_ffmpeg()?);
        command
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-threads",
                "1",
                "-protocol_whitelist",
                "file,pipe",
                "-ss",
            ])
            .arg(seconds(origin))
            .arg("-t")
            .arg(seconds(remaining))
            .arg("-i")
            .arg(path)
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
            .arg(filter)
            .args([
                "-ac",
                &CHANNELS.to_string(),
                "-ar",
                &rate.to_string(),
                "-f",
                "f32le",
                "pipe:1",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        let mut child = MediaChild::start(command, cancellation.clone())?;
        let output = child
            .stdout
            .take()
            .ok_or("Audio decoder output is unavailable.")?;
        let samples = if uniform.is_some() {
            Samples::Uniform(BufReader::new(output))
        } else {
            Samples::Ramped(PitchWarp::new(output, map, origin, rate, start)?)
        };
        Ok(Self {
            child,
            samples,
            next: start,
            end,
            bytes: Vec::new(),
        })
    }

    /// Blocks only on its owning worker and rejects discontinuous reads rather than silently drifting.
    pub fn read(
        &mut self,
        output: &mut [[f32; CHANNELS]],
        start: u64,
        cancellation: &Cancellation,
    ) -> Result<(), String> {
        if start != self.next
            || output.len() > MAX_BLOCK_FRAMES
            || start
                .checked_add(output.len() as u64)
                .is_none_or(|end| end > self.end)
        {
            return Err("The audio block exceeds its admitted forward sample range.".into());
        }
        cancellation.check()?;
        let _deadline = self.child.guard_io();
        match &mut self.samples {
            Samples::Uniform(input) => {
                self.bytes
                    .resize(output.len() * crate::retime::pcm::FRAME_BYTES, 0);
                input
                    .read_exact(&mut self.bytes)
                    .map_err(|error| format!("Audio decode ended early: {error}"))?;
                for (frame, bytes) in output
                    .iter_mut()
                    .zip(self.bytes.chunks_exact(crate::retime::pcm::FRAME_BYTES))
                {
                    for (sample, bytes) in
                        frame.iter_mut().zip(bytes.chunks_exact(size_of::<f32>()))
                    {
                        *sample = f32::from_le_bytes(bytes.try_into().expect("one PCM sample"));
                    }
                }
            }
            Samples::Ramped(warp) => {
                for (index, frame) in output.iter_mut().enumerate() {
                    if index % 256 == 0 {
                        cancellation.check()?;
                    }
                    *frame = warp.sample(start + index as u64)?;
                }
            }
        }
        if output.iter().flatten().any(|sample| !sample.is_finite()) {
            return Err("Audio contains non-finite samples.".into());
        }
        self.next += output.len() as u64;
        Ok(())
    }

    /// Drains the bounded filter tail before waiting, so closing an unread pipe cannot truncate a successful decode.
    pub fn finish(mut self, cancellation: &Cancellation) -> Result<(), String> {
        cancellation.check()?;
        if self.next != self.end {
            return Err("Audio completion requires every admitted output sample.".into());
        }
        let _deadline = self.child.guard_io();
        match &mut self.samples {
            Samples::Uniform(input) => {
                let mut tail = [0; 8192];
                let mut retained = 0;
                loop {
                    cancellation.check()?;
                    let count = input.read(&mut tail).map_err(|error| error.to_string())?;
                    if count == 0 {
                        break;
                    }
                    retained += count;
                    if retained > MAX_BLOCK_FRAMES * crate::retime::pcm::FRAME_BYTES {
                        return Err("Audio decoder exceeded its final PCM extent.".into());
                    }
                }
            }
            Samples::Ramped(input) => input.drain(cancellation)?,
        }
        drop(self.samples);
        self.child.finish()
    }

    /// Preview clip retirement never waits for process teardown on its time-critical mixing worker.
    pub fn retire(self) {
        drop(self.samples);
        self.child.retire();
    }
}

/// Each tempo stage stays in its pitch-preserving range; pad/trim owns the exact final sample count.
fn uniform_filter(mut tempo: f64, frames: u64, rate: u32, remaining: MediaTime) -> String {
    let mut stages = vec![
        format!("atrim=duration={}", seconds(remaining)),
        "asetpts=PTS-STARTPTS".into(),
    ];
    while tempo > 2.0 {
        stages.push("atempo=2".into());
        tempo /= 2.0;
    }
    while tempo < 0.5 {
        stages.push("atempo=0.5".into());
        tempo *= 2.0;
    }
    if tempo != 1.0 {
        stages.push(format!("atempo={tempo:.12}"));
    }
    stages.extend([
        format!("aresample={rate}"),
        format!("apad=whole_len={frames}"),
        format!("atrim=end_sample={frames}"),
    ]);
    stages.join(",")
}

/// Only generated numeric literals enter filter or seek syntax.
fn seconds(at: MediaTime) -> String {
    format!(
        "{}.{:06}",
        at.0 / crate::time::MICROS_PER_SECOND,
        at.0 % crate::time::MICROS_PER_SECOND
    )
}
