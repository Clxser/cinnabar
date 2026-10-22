//! A successful encoder exit must also leave a complete, bounded MP4 container.

use super::*;
use std::io::{Read, Seek, SeekFrom};

const MAX_BOXES: usize = 4096;
mod decoded;
pub(in crate::export) use decoded::verify_media;

/// Checks top-level framing without allocating or scanning movie payloads; cancellation remains live.
pub(in crate::export) fn validate(
    path: &Path,
    maximum: u64,
    cancellation: &Cancellation,
) -> Result<(), String> {
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    let length = file.metadata().map_err(|error| error.to_string())?.len();
    if length > maximum {
        return Err("Export exceeded its output storage limit.".into());
    }
    let mut position = 0_u64;
    let mut seen = [false; 3];
    let mut boxes = 0;
    while position < length {
        cancellation.check()?;
        boxes += 1;
        if boxes > MAX_BOXES || length - position < 8 {
            return Err("The encoder produced an incomplete MP4 container.".into());
        }
        let mut header = [0_u8; 8];
        file.read_exact(&mut header)
            .map_err(|error| error.to_string())?;
        let mut size = u64::from(u32::from_be_bytes(
            header[..4].try_into().expect("box size"),
        ));
        let mut header_size = 8;
        if size == 1 {
            if length - position < 16 {
                return Err("The encoder produced a truncated MP4 box.".into());
            }
            let mut large = [0_u8; 8];
            file.read_exact(&mut large)
                .map_err(|error| error.to_string())?;
            size = u64::from_be_bytes(large);
            header_size = 16;
        } else if size == 0 {
            size = length - position;
        }
        let end = position.checked_add(size).filter(|end| *end <= length);
        let Some(end) = end.filter(|_| size >= header_size) else {
            return Err("The encoder produced invalid MP4 box framing.".into());
        };
        let payload = size - header_size;
        match &header[4..] {
            b"ftyp" if payload >= 8 => seen[0] = true,
            b"moov" if payload > 0 => seen[1] = true,
            b"mdat" if payload > 0 => seen[2] = true,
            _ => {}
        }
        file.seek(SeekFrom::Start(end))
            .map_err(|error| error.to_string())?;
        position = end;
    }
    cancellation.check()?;
    if seen.into_iter().all(|present| present) {
        Ok(())
    } else {
        Err(
            "The encoder did not produce a finalized MP4 movie. No output file was published."
                .into(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Synthetic outer boxes exercise framing admission without pretending to encode a video stream.
    fn boxes() -> Vec<u8> {
        let mut bytes = Vec::new();
        for (kind, payload) in [(b"ftyp", 8_u32), (b"moov", 1), (b"mdat", 1)] {
            bytes.extend((payload + 8).to_be_bytes());
            bytes.extend(kind);
            bytes.resize(bytes.len() + payload as usize, 0);
        }
        bytes
    }

    #[test]
    fn unfinished_truncated_or_over_budget_encoder_files_cannot_publish() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        let token = Cancellation::default();
        assert!(validate(file.path(), 1024, &token).is_err());
        let bytes = boxes();
        file.write_all(&bytes).unwrap();
        assert!(validate(file.path(), 1024, &token).is_ok());
        assert!(validate(file.path(), 8, &token).is_err());
        file.as_file().set_len(bytes.len() as u64 - 1).unwrap();
        assert!(validate(file.path(), 1024, &token).is_err());
        file.as_file().set_len(16).unwrap();
        assert!(validate(file.path(), 1024, &token).is_err());
        token.cancel();
        assert!(validate(file.path(), 1024, &token).is_err());
    }

    #[test]
    fn extended_sizes_and_box_extent_are_checked_without_payload_allocation() {
        let mut bytes = boxes();
        bytes.extend(1_u32.to_be_bytes());
        bytes.extend(b"free");
        bytes.extend(u64::MAX.to_be_bytes());
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(&bytes).unwrap();
        assert!(validate(file.path(), 1024, &Cancellation::default()).is_err());
    }
}
