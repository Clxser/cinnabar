//! Explicit little-endian records with allocation admission before any count-prefixed stream.

use std::io::{Read, Write};

use recording_studio::archive::{ArchiveError, MAX_RECORD_BYTES};

/// Converts bounded binary IO failures into a scene error without assuming an external pathname.
fn error(_: std::io::Error) -> ArchiveError {
    ArchiveError::Invalid("truncated or unwritable native scene object")
}

pub(crate) struct Reader<'a> {
    source: &'a mut dyn Read,
    remaining: usize,
    allocation_limit: usize,
}
impl<'a> Reader<'a> {
    /// Every object is independently bounded, including all its decoded stream allocations.
    pub fn new(source: &'a mut dyn Read) -> Self {
        Self::with_limit(source, MAX_RECORD_BYTES)
    }

    /// Large immutable assets use their own explicit limit; ordinary records keep the journal limit.
    pub fn with_limit(source: &'a mut dyn Read, maximum: usize) -> Self {
        Self {
            source,
            remaining: maximum,
            allocation_limit: maximum,
        }
    }

    /// Charges fixed fields before reading them; count-dependent allocations use this same budget.
    pub fn bytes<const N: usize>(&mut self) -> Result<[u8; N], ArchiveError> {
        self.remaining = self.remaining.checked_sub(N).ok_or(ArchiveError::Invalid(
            "native scene object exceeds its byte limit",
        ))?;
        let mut bytes = [0; N];
        self.source.read_exact(&mut bytes).map_err(error)?;
        Ok(bytes)
    }
    pub fn u32(&mut self) -> Result<u32, ArchiveError> {
        Ok(u32::from_le_bytes(self.bytes()?))
    }

    /// Native tracking identities and floating-point bits use the same little-endian envelope.
    pub fn u64(&mut self) -> Result<u64, ArchiveError> {
        Ok(u64::from_le_bytes(self.bytes()?))
    }

    /// Admits a byte stream before allocating it, then reads it in one bounded operation.
    pub fn blob(&mut self, maximum: usize) -> Result<Vec<u8>, ArchiveError> {
        let count = self.u32()? as usize;
        if count > maximum || count > self.remaining || count > self.allocation_limit {
            return Err(ArchiveError::Invalid(
                "native scene byte stream exceeds its bounds",
            ));
        }
        self.remaining -= count;
        let mut bytes = vec![0; count];
        self.source.read_exact(&mut bytes).map_err(error)?;
        Ok(bytes)
    }

    /// Rejects an oversized array before reserving memory. The constructor checks each record's fields.
    pub fn packed<T, const N: usize>(
        &mut self,
        decode: impl Fn([u32; N]) -> Option<T>,
    ) -> Result<Vec<T>, ArchiveError> {
        self.packed_limit(
            self.allocation_limit / std::mem::size_of::<T>().max(1),
            decode,
        )
    }

    /// Typed owners can impose a tighter element count before an untrusted stream reserves memory.
    pub fn packed_limit<T, const N: usize>(
        &mut self,
        maximum_count: usize,
        decode: impl Fn([u32; N]) -> Option<T>,
    ) -> Result<Vec<T>, ArchiveError> {
        let count = self.u32()? as usize;
        let bytes = count
            .checked_mul(N)
            .and_then(|words| words.checked_mul(std::mem::size_of::<u32>()))
            .ok_or(ArchiveError::Invalid("native scene stream count overflow"))?;
        if count > maximum_count
            || bytes > self.remaining
            || count
                .checked_mul(std::mem::size_of::<T>())
                .is_none_or(|bytes| bytes > self.allocation_limit)
        {
            return Err(ArchiveError::Invalid(
                "native scene stream exceeds its allocation limit",
            ));
        }
        let mut values = Vec::with_capacity(count);
        for _ in 0..count {
            let mut words = [0; N];
            for word in &mut words {
                *word = self.u32()?;
            }
            values.push(
                decode(words).ok_or(ArchiveError::Invalid("invalid packed native scene record"))?,
            );
        }
        Ok(values)
    }
}

/// Writes many packed words in fixed stack-sized batches instead of hashing each word separately.
pub(crate) fn packed<T, const N: usize>(
    output: &mut dyn Write,
    values: &[T],
    encode: impl Fn(&T) -> [u32; N],
) -> Result<(), ArchiveError> {
    let count = u32::try_from(values.len())
        .map_err(|_| ArchiveError::Invalid("native scene stream is too long"))?;
    output.write_all(&count.to_le_bytes()).map_err(error)?;
    let mut buffer = [0_u8; 16 * 1024];
    let mut filled = 0;
    for value in values {
        for word in encode(value) {
            buffer[filled..filled + std::mem::size_of::<u32>()]
                .copy_from_slice(&word.to_le_bytes());
            filled += std::mem::size_of::<u32>();
            if filled == buffer.len() {
                output.write_all(&buffer).map_err(error)?;
                filled = 0;
            }
        }
    }
    output.write_all(&buffer[..filled]).map_err(error)
}

/// Fixed typed fields use the same endian convention as the counted streams.
pub(crate) fn u32(output: &mut dyn Write, value: u32) -> Result<(), ArchiveError> {
    output.write_all(&value.to_le_bytes()).map_err(error)
}

/// Writes one fixed-width identity or floating-point bit pattern.
pub(crate) fn u64(output: &mut dyn Write, value: u64) -> Result<(), ArchiveError> {
    output.write_all(&value.to_le_bytes()).map_err(error)
}

/// Byte arrays retain their exact contents without expanding them into word records.
pub(crate) fn blob(output: &mut dyn Write, value: &[u8]) -> Result<(), ArchiveError> {
    u32(
        output,
        u32::try_from(value.len()).map_err(|_| ArchiveError::Quota)?,
    )?;
    output.write_all(value).map_err(error)
}

/// Object envelopes reject missing or trailing fields; unrecognized data never reaches rendering.
pub(crate) fn finish(input: &mut dyn Read) -> Result<(), ArchiveError> {
    let mut byte = [0_u8; 1];
    if input.read(&mut byte).map_err(error)? != 0 {
        return Err(ArchiveError::Invalid(
            "native scene object has trailing fields",
        ));
    }
    Ok(())
}
