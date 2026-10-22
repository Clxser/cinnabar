//! Fixed-size PCM conversion keeps audio worker writes independent of individual sample count.

use std::io::{self, Write};

const SAMPLE_BYTES: usize = std::mem::size_of::<f32>();
const BATCH_SAMPLES: usize = 1024;

/// Retains little-endian float bits and publishes every fitting sample before a size-limit error.
pub(super) fn append(
    output: &mut impl Write,
    data_bytes: &mut u64,
    maximum: u64,
    samples: impl IntoIterator<Item = f32>,
) -> io::Result<()> {
    let mut samples = samples.into_iter();
    let mut buffer = [0_u8; BATCH_SAMPLES * SAMPLE_BYTES];
    loop {
        let room = (maximum.saturating_sub(*data_bytes) / SAMPLE_BYTES as u64)
            .min(BATCH_SAMPLES as u64) as usize;
        if room == 0 {
            return if samples.next().is_none() {
                Ok(())
            } else if maximum == u64::MAX {
                Err(io::Error::other("recorded audio size overflow"))
            } else {
                Err(io::Error::other(
                    "classic WAV size limit reached; use RF64 for long recordings",
                ))
            };
        }
        let mut length = 0;
        for slot in buffer.chunks_exact_mut(SAMPLE_BYTES).take(room) {
            let Some(sample) = samples.next() else {
                break;
            };
            slot.copy_from_slice(&sample.to_le_bytes());
            length += SAMPLE_BYTES;
        }
        if length == 0 {
            return Ok(());
        }
        output.write_all(&buffer[..length])?;
        *data_bytes += length as u64;
        if length / SAMPLE_BYTES < room {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Counted {
        bytes: Vec<u8>,
        writes: usize,
    }

    impl Write for Counted {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.writes += 1;
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn capture_blocks_use_bounded_writes_and_preserve_every_float_bit() {
        let samples: Vec<_> = (0..BATCH_SAMPLES * 5 + 17)
            .map(|index| f32::from_bits(index as u32 * 617_237))
            .chain([0.0, -0.0, f32::MIN, f32::MAX, f32::INFINITY, f32::NAN])
            .collect();
        let mut output = Counted::default();
        let mut bytes = 0;
        append(&mut output, &mut bytes, u64::MAX, samples.iter().copied()).unwrap();
        let expected: Vec<_> = samples
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect();
        assert_eq!(output.bytes, expected);
        assert_eq!(bytes as usize, expected.len());
        assert_eq!(output.writes, samples.len().div_ceil(BATCH_SAMPLES));
    }

    #[test]
    fn reaching_the_limit_preserves_the_fitting_prefix_and_rejects_only_excess() {
        let mut output = Counted::default();
        let mut bytes = 12;
        append(&mut output, &mut bytes, 20, [1.0, 2.0]).unwrap();
        assert_eq!(bytes, 20);
        append(&mut output, &mut bytes, 20, []).unwrap();
        assert!(append(&mut output, &mut bytes, 20, [3.0]).is_err());
        assert_eq!(output.bytes, [1.0_f32, 2.0].map(f32::to_le_bytes).concat());

        let mut output = Counted::default();
        let mut bytes = 0;
        assert!(append(&mut output, &mut bytes, 10, [1.0, 2.0, 3.0]).is_err());
        assert_eq!(bytes, 8);
        assert_eq!(output.bytes, [1.0_f32, 2.0].map(f32::to_le_bytes).concat());
    }

    #[test]
    fn failed_output_is_reported_without_claiming_a_completed_batch() {
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::new(io::ErrorKind::StorageFull, "fixture"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut bytes = 12;
        let error = append(&mut Broken, &mut bytes, u64::MAX, [0.0; 8]).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::StorageFull);
        assert_eq!(bytes, 12);
    }
}
