//! Count-first tracking envelopes bound actor and label allocation before any decode loop.

use std::{
    collections::BTreeSet,
    io::{Read, Write},
    sync::Arc,
};

use recording_studio::{
    archive::ArchiveError,
    camera::{
        ActorPose, CameraTarget, CameraTargetKind, MAX_CAMERA_TARGETS, MAX_TARGET_NAME_BYTES,
    },
};

use super::Delta;
use crate::binary::{self, Reader};

const MAGIC: [u8; 8] = *b"CINTRK02";
const LEGACY_MAGIC: [u8; 8] = *b"CINTRK01";

#[cfg(test)]
mod tests;

/// Exact field binding forces a codec update when the shared target contract changes.
pub(super) fn write(output: &mut dyn Write, delta: &Delta) -> Result<(), ArchiveError> {
    output.write_all(&MAGIC).map_err(io_error)?;
    binary::u32(output, delta.upserts.len() as u32)?;
    for target in &delta.upserts {
        target
            .validate()
            .map_err(|_| ArchiveError::Invalid("invalid captured camera target"))?;
        let CameraTarget {
            runtime_id,
            unique_id,
            spawn_revision,
            name,
            kind,
            entity_type,
            pose,
            teleported,
        } = target;
        for value in [*runtime_id, *unique_id as u64, *spawn_revision] {
            binary::u64(output, value)?;
        }
        let ActorPose {
            position,
            velocity,
            yaw,
            pitch,
            roll,
            eye_offset,
        } = pose;
        for value in position.iter().chain(velocity).chain([yaw, pitch, roll]) {
            binary::u64(output, value.to_bits())?;
        }
        binary::u32(output, u32::from(eye_offset.is_some()))?;
        for axis in eye_offset.unwrap_or([0.0; 3]) {
            binary::u64(output, axis.to_bits())?;
        }
        binary::u32(output, u32::from(*teleported))?;
        binary::blob(output, name.as_bytes())?;
        binary::u32(output, kind.code())?;
        binary::blob(output, entity_type.as_deref().unwrap_or("").as_bytes())?;
    }
    binary::packed(output, &delta.removed, |id| {
        [*id as u32, (*id >> 32) as u32]
    })
}

/// Counts, UTF-8, duplicate identities, non-finite values and trailing bytes fail before publication.
pub(super) fn read(input: &mut dyn Read) -> Result<Delta, ArchiveError> {
    let mut reader = Reader::new(input);
    let magic = reader.bytes::<8>()?;
    if magic != MAGIC && magic != LEGACY_MAGIC {
        return Err(ArchiveError::Invalid("unsupported camera target envelope"));
    }
    let count = reader.u32()? as usize;
    if count > MAX_CAMERA_TARGETS {
        return Err(ArchiveError::Quota);
    }
    let mut upserts = Vec::with_capacity(count);
    let mut seen = BTreeSet::new();
    for _ in 0..count {
        let runtime_id = reader.u64()?;
        let unique_id = reader.u64()? as i64;
        let spawn_revision = reader.u64()?;
        let mut values = [0.0; 9];
        for value in &mut values {
            *value = f64::from_bits(reader.u64()?);
        }
        let has_eye = boolean(reader.u32()?)?;
        let mut eye = [0.0; 3];
        for axis in &mut eye {
            *axis = f64::from_bits(reader.u64()?);
        }
        if !has_eye && eye.iter().any(|axis| axis.to_bits() != 0) {
            return Err(ArchiveError::Invalid(
                "noncanonical absent target eye offset",
            ));
        }
        let teleported = boolean(reader.u32()?)?;
        let name = String::from_utf8(reader.blob(MAX_TARGET_NAME_BYTES)?)
            .map_err(|_| ArchiveError::Invalid("camera target name is not UTF-8"))?;
        let (kind, entity_type) = if magic == MAGIC {
            let kind = CameraTargetKind::from_code(reader.u32()?).ok_or(ArchiveError::Invalid(
                "invalid camera target classification",
            ))?;
            let entity_type = String::from_utf8(reader.blob(MAX_TARGET_NAME_BYTES)?)
                .map_err(|_| ArchiveError::Invalid("camera target entity type is not UTF-8"))?;
            (
                kind,
                (!entity_type.is_empty()).then(|| Arc::from(entity_type)),
            )
        } else {
            (CameraTargetKind::Unknown, None)
        };
        let target = CameraTarget {
            runtime_id,
            unique_id,
            spawn_revision,
            name: Arc::from(name),
            kind,
            entity_type,
            teleported,
            pose: ActorPose {
                position: values[..3].try_into().expect("three position axes"),
                velocity: values[3..6].try_into().expect("three velocity axes"),
                yaw: values[6],
                pitch: values[7],
                roll: values[8],
                eye_offset: has_eye.then_some(eye),
            },
        };
        target
            .validate()
            .map_err(|_| ArchiveError::Invalid("invalid recorded camera target"))?;
        if !seen.insert(runtime_id) {
            return Err(ArchiveError::Invalid("duplicate camera target upsert"));
        }
        upserts.push(target);
    }
    let removed = reader.packed_limit(MAX_CAMERA_TARGETS, |words: [u32; 2]| {
        Some(u64::from(words[0]) | (u64::from(words[1]) << 32))
    })?;
    for &id in &removed {
        if !seen.insert(id) {
            return Err(ArchiveError::Invalid(
                "duplicate or contradictory camera target delta",
            ));
        }
    }
    drop(reader);
    binary::finish(input)?;
    Ok(Delta { upserts, removed })
}

/// Flags have one canonical representation rather than accepting arbitrary nonzero integers.
fn boolean(value: u32) -> Result<bool, ArchiveError> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(ArchiveError::Invalid("invalid camera target flag")),
    }
}

/// Tracking IO failures remain separate from live-world decoding.
fn io_error(_: std::io::Error) -> ArchiveError {
    ArchiveError::Invalid("truncated or unwritable camera target data")
}
