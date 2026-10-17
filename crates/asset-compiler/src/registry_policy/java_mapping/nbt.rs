//! Reads the Java block tags from the generator's gzip-compressed, big-endian network tag
//! export: a root compound of registries, `minecraft:block` mapping tag ids to integer
//! arrays of Java block registry ids.

use super::Result;
use flate2::read::GzDecoder;
use std::io::Read;

const BLOCK_REGISTRY: &str = "minecraft:block";
const MAX_INFLATED_BYTES: u64 = 16 * 1024 * 1024;
const MAX_DEPTH: usize = 32;

const END: u8 = 0;
const BYTE: u8 = 1;
const SHORT: u8 = 2;
const INT: u8 = 3;
const LONG: u8 = 4;
const FLOAT: u8 = 5;
const DOUBLE: u8 = 6;
const BYTE_ARRAY: u8 = 7;
const STRING: u8 = 8;
const LIST: u8 = 9;
const COMPOUND: u8 = 10;
const INT_ARRAY: u8 = 11;
const LONG_ARRAY: u8 = 12;

/// Every block tag with its member registry ids, in file order.
pub(super) fn block_tags(compressed: &[u8]) -> Result<Vec<(String, Vec<i32>)>> {
    let mut bytes = Vec::new();
    GzDecoder::new(compressed)
        .take(MAX_INFLATED_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_INFLATED_BYTES {
        return Err("network tag export exceeds its size limit".into());
    }
    let mut reader = Reader { bytes: &bytes };
    if reader.u8()? != COMPOUND {
        return Err("network tag export has no root compound".into());
    }
    reader.string()?;
    let mut tags = None;
    loop {
        let kind = reader.u8()?;
        if kind == END {
            break;
        }
        let name = reader.string()?;
        if kind == COMPOUND && name == BLOCK_REGISTRY {
            tags = Some(reader.int_lists()?);
        } else {
            reader.skip(kind, 0)?;
        }
    }
    if !reader.bytes.is_empty() {
        return Err("trailing bytes after the network tag export".into());
    }
    tags.ok_or_else(|| format!("network tag export has no {BLOCK_REGISTRY} registry").into())
}

struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        if self.bytes.len() < count {
            return Err("truncated network tag export".into());
        }
        let (head, tail) = self.bytes.split_at(count);
        self.bytes = tail;
        Ok(head)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_be_bytes(self.take(4)?.try_into()?))
    }

    fn length(&mut self) -> Result<usize> {
        usize::try_from(self.i32()?).map_err(|_| "negative NBT length".into())
    }

    fn string(&mut self) -> Result<String> {
        let length = u16::from_be_bytes(self.take(2)?.try_into()?);
        Ok(std::str::from_utf8(self.take(usize::from(length))?)?.to_owned())
    }

    /// A compound whose values are int arrays (or lists of ints).
    fn int_lists(&mut self) -> Result<Vec<(String, Vec<i32>)>> {
        let mut entries = Vec::new();
        loop {
            let kind = self.u8()?;
            if kind == END {
                return Ok(entries);
            }
            let name = self.string()?;
            let values = match kind {
                INT_ARRAY => (0..self.length()?)
                    .map(|_| self.i32())
                    .collect::<Result<_>>()?,
                LIST => {
                    let element = self.u8()?;
                    let count = self.length()?;
                    if element != INT && count > 0 {
                        return Err(format!("block tag {name} is not a list of ints").into());
                    }
                    (0..count).map(|_| self.i32()).collect::<Result<_>>()?
                }
                _ => return Err(format!("block tag {name} is not an int array").into()),
            };
            entries.push((name, values));
        }
    }

    fn skip(&mut self, kind: u8, depth: usize) -> Result<()> {
        if depth > MAX_DEPTH {
            return Err("network tag export nests too deeply".into());
        }
        match kind {
            BYTE => {
                self.take(1)?;
            }
            SHORT => {
                self.take(2)?;
            }
            INT | FLOAT => {
                self.take(4)?;
            }
            LONG | DOUBLE => {
                self.take(8)?;
            }
            BYTE_ARRAY => {
                let count = self.length()?;
                self.take(count)?;
            }
            STRING => {
                self.string()?;
            }
            LIST => {
                let element = self.u8()?;
                for _ in 0..self.length()? {
                    self.skip(element, depth + 1)?;
                }
            }
            COMPOUND => loop {
                let kind = self.u8()?;
                if kind == END {
                    break;
                }
                self.string()?;
                self.skip(kind, depth + 1)?;
            },
            INT_ARRAY => {
                let count = self.length()?;
                self.take(count.checked_mul(4).ok_or("NBT length overflow")?)?;
            }
            LONG_ARRAY => {
                let count = self.length()?;
                self.take(count.checked_mul(8).ok_or("NBT length overflow")?)?;
            }
            _ => return Err(format!("unknown NBT tag type {kind}").into()),
        }
        Ok(())
    }
}
