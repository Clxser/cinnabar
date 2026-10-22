//! A streaming 32-bit float WAV writer for captured game audio.

use std::{
    fs::File,
    io::{self, BufWriter, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

const HEADER_BYTES: u32 = 44;
const FLOAT_FORMAT: u16 = 3;
const RF64_HEADER_BYTES: u64 = 80;

pub struct WavWriter {
    out: BufWriter<File>,
    path: PathBuf,
    data_bytes: u64,
    rf64_channels: Option<u16>,
}

impl WavWriter {
    /// Trims hardware prebuffer beyond a stopped video's exact interleaved sample duration.
    pub fn trim_samples(&mut self, maximum: u64) -> io::Result<()> {
        let bytes = maximum
            .checked_mul(4)
            .ok_or_else(|| io::Error::other("audio sample count overflow"))?;
        if bytes >= self.data_bytes {
            return Ok(());
        }
        self.out.flush()?;
        let header = if self.rf64_channels.is_some() {
            RF64_HEADER_BYTES
        } else {
            u64::from(HEADER_BYTES)
        };
        self.out.get_ref().set_len(header + bytes)?;
        self.out.seek(SeekFrom::End(0))?;
        self.data_bytes = bytes;
        Ok(())
    }
    pub fn create(path: &Path, channels: u16, rate: u32) -> io::Result<Self> {
        let mut out = BufWriter::new(File::create(path)?);
        let block = channels * 4;
        out.write_all(b"RIFF")?;
        out.write_all(&(HEADER_BYTES - 8).to_le_bytes())?;
        out.write_all(b"WAVEfmt ")?;
        out.write_all(&16_u32.to_le_bytes())?;
        out.write_all(&FLOAT_FORMAT.to_le_bytes())?;
        out.write_all(&channels.to_le_bytes())?;
        out.write_all(&rate.to_le_bytes())?;
        out.write_all(&(rate * u32::from(block)).to_le_bytes())?;
        out.write_all(&block.to_le_bytes())?;
        out.write_all(&32_u16.to_le_bytes())?;
        out.write_all(b"data")?;
        out.write_all(&0_u32.to_le_bytes())?;
        Ok(Self {
            out,
            path: path.to_owned(),
            data_bytes: 0,
            rf64_channels: None,
        })
    }

    /// RF64 keeps long native recordings valid beyond the classic WAV four-gigabyte limit.
    pub fn create_rf64(path: &Path, channels: u16, rate: u32) -> io::Result<Self> {
        if channels == 0 || channels > 32 || rate == 0 || rate > 768_000 {
            return Err(io::Error::other("invalid recording audio format"));
        }
        let mut out = BufWriter::new(File::create(path)?);
        let block = channels * 4;
        out.write_all(b"RF64")?;
        out.write_all(&u32::MAX.to_le_bytes())?;
        out.write_all(b"WAVEds64")?;
        out.write_all(&28_u32.to_le_bytes())?;
        out.write_all(&[0; 24])?;
        out.write_all(&0_u32.to_le_bytes())?;
        out.write_all(b"fmt ")?;
        out.write_all(&16_u32.to_le_bytes())?;
        out.write_all(&FLOAT_FORMAT.to_le_bytes())?;
        out.write_all(&channels.to_le_bytes())?;
        out.write_all(&rate.to_le_bytes())?;
        out.write_all(&(rate * u32::from(block)).to_le_bytes())?;
        out.write_all(&block.to_le_bytes())?;
        out.write_all(&32_u16.to_le_bytes())?;
        out.write_all(b"data")?;
        out.write_all(&u32::MAX.to_le_bytes())?;
        Ok(Self {
            out,
            path: path.to_owned(),
            data_bytes: 0,
            rf64_channels: Some(channels),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Appends interleaved samples.
    pub fn write(&mut self, samples: impl IntoIterator<Item = f32>) -> io::Result<()> {
        for sample in samples {
            let next = self
                .data_bytes
                .checked_add(4)
                .ok_or_else(|| io::Error::other("recorded audio size overflow"))?;
            if self.rf64_channels.is_none() && next > u64::from(u32::MAX - (HEADER_BYTES - 8)) {
                return Err(io::Error::other(
                    "classic WAV size limit reached; use RF64 for long recordings",
                ));
            }
            self.out.write_all(&sample.to_le_bytes())?;
            self.data_bytes = next;
        }
        Ok(())
    }

    /// Patches the chunk sizes and flushes.
    pub fn finish(mut self) -> io::Result<PathBuf> {
        self.out.flush()?;
        let mut file = self.out.into_inner().map_err(io::Error::other)?;
        if let Some(channels) = self.rf64_channels {
            file.seek(SeekFrom::Start(20))?;
            file.write_all(&(RF64_HEADER_BYTES - 8 + self.data_bytes).to_le_bytes())?;
            file.write_all(&self.data_bytes.to_le_bytes())?;
            file.write_all(&(self.data_bytes / (u64::from(channels) * 4)).to_le_bytes())?;
        } else {
            file.seek(SeekFrom::Start(4))?;
            file.write_all(&(HEADER_BYTES - 8 + self.data_bytes as u32).to_le_bytes())?;
            file.seek(SeekFrom::Start(40))?;
            file.write_all(&(self.data_bytes as u32).to_le_bytes())?;
        }
        file.sync_all()?;
        Ok(self.path)
    }
}

/// Interleaved samples owed for output frame `frame` at `fps`, so every second carries
/// exactly `rate` sample frames even when `rate / fps` is fractional.
pub fn samples_for_frame(rate: u32, channels: u16, fps: u32, frame: u64) -> usize {
    let at = |frame: u64| u128::from(frame) * u128::from(rate) / u128::from(fps.max(1));
    let frames = at(frame + 1) - at(frame);
    usize::try_from(frames).unwrap_or(usize::MAX) * usize::from(channels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_sizes_are_patched_on_finish() {
        let path = std::env::temp_dir().join(format!("cinnabar-wav-{}.wav", std::process::id()));
        let mut wav = WavWriter::create(&path, 2, 48_000).unwrap();
        wav.write([0.5, -0.5, 0.25, -0.25]).unwrap();
        let path = wav.finish().unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes.len(), 44 + 16);
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 36 + 16);
        assert_eq!(u16::from_le_bytes(bytes[20..22].try_into().unwrap()), 3);
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 16);
        assert_eq!(f32::from_le_bytes(bytes[44..48].try_into().unwrap()), 0.5);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn audio_per_frame_sums_to_the_rate() {
        let total: usize = (0..30)
            .map(|frame| samples_for_frame(44_100, 2, 30, frame))
            .sum();
        assert_eq!(total, 44_100 * 2);
        assert_eq!(samples_for_frame(48_000, 2, 60, 0), 1_600);
    }
}
