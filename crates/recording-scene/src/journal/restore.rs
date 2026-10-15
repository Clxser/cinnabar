//! One typed terrain admission path serves checkpoint seeks and incremental forward playback.

use recording_studio::archive::{ArchiveError, RecordKind, ReplayRecord};

use crate::scene::{ChunkId, MAX_LINK_BYTES, SceneLoader, TerrainRecord, TerrainScene};

/// Applies a validated record to a private draft and reports only a binding that actually changed.
pub(crate) fn apply_record(
    record: ReplayRecord<'_>,
    scene: &mut TerrainScene,
    loader: &mut SceneLoader,
    cancelled: &impl Fn() -> bool,
) -> Result<Option<ChunkId>, ArchiveError> {
    match record.kind {
        RecordKind::CheckpointStart => scene.clear(),
        RecordKind::CheckpointEnd => {}
        RecordKind::CheckpointPart | RecordKind::Scene => {
            if record.payload.len() > MAX_LINK_BYTES {
                return Err(ArchiveError::Invalid(
                    "terrain change exceeds its byte limit",
                ));
            }
            let change: TerrainRecord = serde_json::from_slice(record.payload)?;
            match change {
                TerrainRecord::Upsert { chunk } => {
                    chunk.key.origin()?;
                    let key = chunk.key;
                    if scene.chunk(key).is_some_and(|old| old.binding == chunk) {
                        return Ok(None);
                    }
                    scene.upsert(loader.load_cancellable(chunk, cancelled)?)?;
                    return Ok(Some(key));
                }
                TerrainRecord::Remove { key } if record.kind == RecordKind::Scene => {
                    key.origin()?;
                    if scene.chunk(key).is_some() {
                        scene.remove(key);
                        return Ok(Some(key));
                    }
                }
                TerrainRecord::Remove { .. } => {
                    return Err(ArchiveError::Invalid(
                        "terrain checkpoint contains an unload",
                    ));
                }
            }
        }
        _ => {
            return Err(ArchiveError::Invalid(
                "unexpected record in native terrain track",
            ));
        }
    }
    Ok(None)
}
