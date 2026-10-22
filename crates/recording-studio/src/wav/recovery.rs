//! Recovers only physical complete PCM frames in the exact RF64 layout emitted by our writer.

use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use tempfile::NamedTempFile;

use super::{RF64_HEADER_BYTES, WavWriter, validate_rf64_format};
use crate::{
    MediaTime,
    export::Cancellation,
    time::{MAX_MEDIA_MICROS, MICROS_PER_SECOND},
};

pub(crate) struct PreparedAudio {
    staged: NamedTempFile,
    pub duration: MediaTime,
    pub frames: u64,
    pub rate: u32,
    pub discarded_bytes: u64,
    pub unfinished: bool,
    pub truncated: bool,
}

/// Publication is rolled back unless the caller successfully assembles the recovered project.
pub(crate) struct PublishedAudio {
    pub path: PathBuf,
    retained: bool,
}

impl PublishedAudio {
    /// Once the project owns the source, only an explicit user action may remove recovered media.
    pub fn retain(mut self) {
        self.retained = true;
    }
}

impl Drop for PublishedAudio {
    fn drop(&mut self) {
        if !self.retained {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

impl PreparedAudio {
    /// Draft admission can use the staged path before any recoverable file is published.
    pub fn path(&self) -> &Path {
        self.staged.path()
    }

    /// No-clobber publication preserves prior recovery attempts and every original take file.
    pub fn publish(
        self,
        directory: &Path,
        cancellation: &Cancellation,
    ) -> Result<PublishedAudio, String> {
        let mut staged = self.staged;
        let mut ordinal = 1_u64;
        loop {
            cancellation.check()?;
            let path = directory.join(format!("Recovered game audio {ordinal}.wav"));
            match staged.persist_noclobber(&path) {
                Ok(file) => {
                    let published = PublishedAudio {
                        path,
                        retained: false,
                    };
                    file.sync_all().map_err(|error| error.to_string())?;
                    #[cfg(unix)]
                    File::open(directory)
                        .and_then(|file| file.sync_all())
                        .map_err(|error| error.to_string())?;
                    return Ok(published);
                }
                Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                    staged = error.file;
                    ordinal = ordinal
                        .checked_add(1)
                        .ok_or("Recovered audio names are exhausted.")?;
                }
                Err(error) => return Err(error.error.to_string()),
            }
        }
    }
}

struct Format {
    channels: u16,
    rate: u32,
    declared_bytes: Option<u64>,
}

/// Zero ds64 fields identify an unfinalized writer; nonzero sizes must agree with each other.
fn format(header: &[u8; RF64_HEADER_BYTES as usize]) -> Result<Format, String> {
    let u16_at = |at| u16::from_le_bytes(header[at..at + 2].try_into().expect("header field"));
    let u32_at = |at| u32::from_le_bytes(header[at..at + 4].try_into().expect("header field"));
    let u64_at = |at| u64::from_le_bytes(header[at..at + 8].try_into().expect("header field"));
    let channels = u16_at(58);
    let rate = u32_at(60);
    validate_rf64_format(channels, rate).map_err(|error| error.to_string())?;
    let block = channels * 4;
    if &header[0..4] != b"RF64"
        || u32_at(4) != u32::MAX
        || &header[8..16] != b"WAVEds64"
        || u32_at(16) != 28
        || u32_at(44) != 0
        || &header[48..52] != b"fmt "
        || u32_at(52) != 16
        || u16_at(56) != super::FLOAT_FORMAT
        || u32_at(64) != rate * u32::from(block)
        || u16_at(68) != block
        || u16_at(70) != 32
        || &header[72..76] != b"data"
        || u32_at(76) != u32::MAX
    {
        return Err("The audio sidecar is not the capture writer's RF64 float PCM format.".into());
    }
    let (riff, bytes, frames) = (u64_at(20), u64_at(28), u64_at(36));
    let declared_bytes = if [riff, bytes, frames] == [0; 3] {
        None
    } else {
        if bytes % u64::from(block) != 0
            || frames != bytes / u64::from(block)
            || riff
                != (RF64_HEADER_BYTES - 8)
                    .checked_add(bytes)
                    .ok_or("RF64 sizes overflow.")?
        {
            return Err(
                "The RF64 audio size fields are inconsistent or only partially finalized.".into(),
            );
        }
        Some(bytes)
    };
    Ok(Format {
        channels,
        rate,
        declared_bytes,
    })
}

/// This worker operation reads a fixed physical prefix and never patches the source's unfinished header.
pub(crate) fn prepare(
    source: &Path,
    prefix: MediaTime,
    cancellation: &Cancellation,
) -> Result<PreparedAudio, String> {
    prepare_checked(source, prefix, &|| cancellation.check())
}

/// Caller-supplied cancellation boundaries make mid-copy cleanup deterministic to exercise.
fn prepare_checked(
    source: &Path,
    prefix: MediaTime,
    check: &impl Fn() -> Result<(), String>,
) -> Result<PreparedAudio, String> {
    check()?;
    if prefix == MediaTime::ZERO || prefix.0 > MAX_MEDIA_MICROS {
        return Err("Audio recovery needs a nonempty verified scene prefix.".into());
    }
    if !std::fs::symlink_metadata(source)
        .map_err(|error| error.to_string())?
        .is_file()
    {
        return Err("The audio sidecar is not a regular file.".into());
    }
    let mut input = File::open(source).map_err(|error| error.to_string())?;
    let physical = input.metadata().map_err(|error| error.to_string())?.len();
    let mut header = [0_u8; RF64_HEADER_BYTES as usize];
    input
        .read_exact(&mut header)
        .map_err(|error| format!("The RF64 audio header is incomplete: {error}"))?;
    let format = format(&header)?;
    let physical_data = physical
        .checked_sub(RF64_HEADER_BYTES)
        .ok_or("The RF64 audio header is incomplete.")?;
    if format
        .declared_bytes
        .is_some_and(|bytes| bytes < physical_data)
    {
        return Err("Unexpected bytes follow the finalized RF64 payload.".into());
    }
    let frame_bytes = u64::from(format.channels) * 4;
    let available = physical_data / frame_bytes;
    let requested = u64::try_from(
        u128::from(prefix.0) * u128::from(format.rate) / u128::from(MICROS_PER_SECOND),
    )
    .map_err(|_| "Audio duration overflows.")?;
    let frames = available.min(requested);
    if frames == 0 {
        return Err(
            "No complete recorded PCM frame survives inside the verified scene prefix.".into(),
        );
    }
    let bytes = frames * frame_bytes;
    let directory = source
        .parent()
        .ok_or("The audio sidecar has no parent directory.")?;
    let staged = tempfile::Builder::new()
        .prefix(".recover-audio-")
        .suffix(".wav")
        .tempfile_in(directory)
        .map_err(|error| error.to_string())?;
    let file = staged
        .as_file()
        .try_clone()
        .map_err(|error| error.to_string())?;
    let mut output = WavWriter::rf64_file(file, staged.path(), format.channels, format.rate)
        .map_err(|error| error.to_string())?;
    let mut buffer = vec![0; crate::audio::MAX_BLOCK_FRAMES * frame_bytes as usize];
    let mut remaining = bytes;
    while remaining != 0 {
        check()?;
        let length = remaining.min(buffer.len() as u64) as usize;
        input
            .read_exact(&mut buffer[..length])
            .map_err(|error| error.to_string())?;
        if buffer[..length]
            .chunks_exact(4)
            .any(|sample| !f32::from_le_bytes(sample.try_into().expect("PCM float")).is_finite())
        {
            return Err("Recorded audio contains a non-finite PCM sample.".into());
        }
        output
            .out
            .write_all(&buffer[..length])
            .map_err(|error| error.to_string())?;
        output.data_bytes += length as u64;
        remaining -= length as u64;
    }
    check()?;
    input
        .seek(SeekFrom::Start(0))
        .map_err(|error| error.to_string())?;
    let mut current = [0; RF64_HEADER_BYTES as usize];
    input
        .read_exact(&mut current)
        .map_err(|error| error.to_string())?;
    if current != header || input.metadata().map_err(|error| error.to_string())?.len() != physical {
        return Err(
            "The audio sidecar changed during recovery; stop capture before retrying.".into(),
        );
    }
    output.finish().map_err(|error| error.to_string())?;
    check()?;
    let duration = MediaTime(
        (u128::from(frames) * u128::from(MICROS_PER_SECOND) / u128::from(format.rate)) as u64,
    );
    if duration == MediaTime::ZERO {
        return Err(
            "The recovered PCM interval is shorter than the project clock can represent.".into(),
        );
    }
    Ok(PreparedAudio {
        staged,
        duration,
        frames,
        rate: format.rate,
        discarded_bytes: physical_data - bytes,
        unfinished: format.declared_bytes.is_none(),
        truncated: format
            .declared_bytes
            .is_some_and(|bytes| bytes > physical_data),
    })
}

#[cfg(test)]
mod tests;
