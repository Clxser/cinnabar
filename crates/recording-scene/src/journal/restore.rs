//! One typed terrain admission path serves checkpoint seeks and incremental forward playback.

use recording_studio::archive::{ArchiveError, RecordKind, ReplayRecord};

use crate::scene::{
    ChunkId, MAX_LINK_BYTES, SceneLoader, ScenePalette, TerrainRecord, TerrainScene,
};

/// Applies a validated record to a private draft and reports only a binding that actually changed.
pub(crate) fn apply_record(
    record: ReplayRecord<'_>,
    scene: &mut TerrainScene,
    loader: &mut SceneLoader,
    cancelled: &impl Fn() -> bool,
) -> Result<Option<ChunkId>, ArchiveError> {
    match record.kind {
        RecordKind::CheckpointStart => scene.clear(),
        RecordKind::CheckpointEnd => {
            if scene.active_palette().is_none() {
                return Err(ArchiveError::Invalid(
                    "terrain checkpoint has no selected palette",
                ));
            }
        }
        RecordKind::CheckpointPart | RecordKind::Scene => {
            if record.payload.len() > MAX_LINK_BYTES {
                return Err(ArchiveError::Invalid(
                    "terrain change exceeds its byte limit",
                ));
            }
            let change: TerrainRecord = serde_json::from_slice(record.payload)?;
            match change {
                TerrainRecord::SelectPalette { palette } => {
                    if scene
                        .active_palette()
                        .is_none_or(|current| current.id != palette)
                    {
                        scene.select_palette(ScenePalette {
                            id: palette,
                            palette: loader.palette(palette, cancelled)?,
                        })?;
                    }
                }
                TerrainRecord::Upsert { chunk } => {
                    if scene.active_palette().is_none() {
                        return Err(ArchiveError::Invalid(
                            "terrain mesh precedes palette selection",
                        ));
                    }
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
