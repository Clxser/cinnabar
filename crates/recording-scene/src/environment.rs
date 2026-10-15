//! Prepared environment snapshots and immutable sky assets for independent native playback.

use std::{
    io::{Read, Write},
    path::Path,
    sync::Arc,
};

use assets::{PackSkybox, RuntimeAtmosphereAssets, SkyboxDimension};
use recording_studio::{
    MediaTime,
    archive::{ArchiveError, RecordKind, RecordingCursor, RecordingReader, ReplayRecord},
    object_store::{AssetId, AssetReader},
};
use render_model::environment_snapshot::{AtmosphereSnapshot, LightmapSnapshot};

use crate::binary::{self, Reader};

mod capture;
mod sky_assets;
pub use capture::{EnvironmentCapture, PreparedEnvironmentAssets};

pub const ENVIRONMENT_TRACK_SCHEMA: &str = "cinnabar.environment/1";
pub const ENVIRONMENT_DIRECTORY: &str = "environment.cinreplay";
const FRAME_MAGIC: [u8; 8] = *b"CINEFR01";
const MAX_ASSET_BYTES: u64 = 128 * 1024 * 1024;
type Skyboxes = [PackSkybox; SkyboxDimension::ALL.len()];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnvironmentState {
    pub atmosphere: AtmosphereSnapshot,
    pub lightmap: LightmapSnapshot,
    pub fullbright: bool,
    pub animation_tick: u32,
    pub animation_partial: f32,
}

impl EnvironmentState {
    /// Uses each owner's admission before a recorded value becomes a native draw input.
    fn valid(self) -> bool {
        AtmosphereSnapshot::from_words(self.atmosphere.words()).is_some()
            && LightmapSnapshot::from_words(self.lightmap.words()).is_some()
            && self.animation_partial.is_finite()
            && (0.0..1.0).contains(&self.animation_partial)
    }
}

#[derive(Clone)]
pub struct FrozenEnvironment {
    /// Includes runtime overrides and skyboxes, so replay GPU resources cannot reuse a stale base identity.
    pub assets_identity: AssetId,
    pub state: EnvironmentState,
    pub atmosphere: Arc<RuntimeAtmosphereAssets>,
    pub skyboxes: Option<Arc<Skyboxes>>,
}

/// Writes one small state record. Immutable pixel objects are addressed separately.
fn write_frame(
    output: &mut dyn Write,
    assets: AssetId,
    state: EnvironmentState,
) -> Result<(), ArchiveError> {
    if !state.valid() {
        return Err(ArchiveError::Invalid(
            "invalid prepared recording environment",
        ));
    }
    output
        .write_all(&FRAME_MAGIC)
        .and_then(|()| output.write_all(&assets.0))
        .map_err(io_error)?;
    binary::packed(output, &[state.atmosphere], |value| value.words())?;
    binary::packed(output, &[state.lightmap], |value| value.words())?;
    binary::u32(output, u32::from(state.fullbright))?;
    binary::u32(output, state.animation_tick)?;
    binary::u32(output, state.animation_partial.to_bits())
}

/// Fixed records use counted arrays so truncated, appended or incorrectly sized states are rejected.
fn read_frame(input: &mut dyn Read) -> Result<(AssetId, EnvironmentState), ArchiveError> {
    let mut reader = Reader::new(input);
    if reader.bytes::<8>()? != FRAME_MAGIC {
        return Err(ArchiveError::Invalid(
            "unsupported environment frame schema",
        ));
    }
    let assets = AssetId(reader.bytes::<32>()?);
    let atmosphere = reader.packed(AtmosphereSnapshot::from_words)?;
    let lightmap = reader.packed(LightmapSnapshot::from_words)?;
    if atmosphere.len() != 1 || lightmap.len() != 1 {
        return Err(ArchiveError::Invalid(
            "environment frame must contain one atmosphere and lightmap",
        ));
    }
    let fullbright = match reader.u32()? {
        0 => false,
        1 => true,
        _ => return Err(ArchiveError::Invalid("invalid environment fullbright flag")),
    };
    let state = EnvironmentState {
        atmosphere: atmosphere[0],
        lightmap: lightmap[0],
        fullbright,
        animation_tick: reader.u32()?,
        animation_partial: f32::from_bits(reader.u32()?),
    };
    drop(reader);
    binary::finish(input)?;
    if !state.valid() {
        return Err(ArchiveError::Invalid("invalid recorded environment state"));
    }
    Ok((assets, state))
}

pub struct EnvironmentRecording {
    cursor: RecordingCursor,
    store: AssetReader,
    assets: Option<(AssetId, Arc<sky_assets::LoadedAssets>)>,
    current: Option<FrozenEnvironment>,
}

impl EnvironmentRecording {
    /// Opens only this owner's scene schema, independently of any current session or selected shader mode.
    pub fn open(parent: &Path) -> Result<Self, ArchiveError> {
        let root = parent.join(ENVIRONMENT_DIRECTORY);
        let reader = RecordingReader::open(&root)?;
        if reader.info().scene_schema != ENVIRONMENT_TRACK_SCHEMA {
            return Err(ArchiveError::Invalid(
                "recording has no supported environment track",
            ));
        }
        Ok(Self {
            cursor: reader.into_cursor(),
            store: AssetReader::new(&root),
            assets: None,
            current: None,
        })
    }

    /// Reconstructs source state on a worker. An obsolete seek cannot publish partially decoded assets.
    pub fn restore(
        &mut self,
        at: MediaTime,
        cancelled: impl Fn() -> bool,
    ) -> Result<FrozenEnvironment, ArchiveError> {
        let mut latest = None;
        let result = self
            .cursor
            .seek(at, &cancelled, |record| latest_state(record, &mut latest))
            .and_then(|()| {
                latest.ok_or(ArchiveError::Invalid(
                    "environment track has no prepared frame",
                ))
            })
            .and_then(|(id, state)| self.load_state(id, state, &cancelled));
        match result {
            Ok(frame) => {
                self.current = Some(frame.clone());
                Ok(frame)
            }
            Err(error) => {
                self.cursor.invalidate();
                Err(error)
            }
        }
    }

    /// Keeps the last prepared state while consuming only newly crossed journal records.
    pub fn advance_to(
        &mut self,
        at: MediaTime,
        cancelled: impl Fn() -> bool,
    ) -> Result<&FrozenEnvironment, ArchiveError> {
        let mut latest = None;
        let result = self
            .cursor
            .advance_to(at, &cancelled, |record| latest_state(record, &mut latest))
            .and_then(|()| {
                latest
                    .map(|(id, state)| self.load_state(id, state, &cancelled))
                    .transpose()
            });
        match result {
            Ok(Some(frame)) => self.current = Some(frame),
            Ok(None) => {}
            Err(error) => {
                self.cursor.invalidate();
                return Err(error);
            }
        }
        self.current.as_ref().ok_or(ArchiveError::Invalid(
            "environment playback needs a successful seek",
        ))
    }

    /// A failed sibling track prevents further publication until the whole scene seeks again.
    pub fn invalidate(&mut self) {
        self.cursor.invalidate();
    }

    /// Interrupted tracks expose only the duration represented by committed segments.
    pub fn duration(&self) -> MediaTime {
        self.cursor.reader().duration()
    }

    /// Track metadata stays available for cross-track take admission without reopening files.
    pub fn info(&self) -> &recording_studio::archive::RecordingInfo {
        self.cursor.reader().info()
    }

    /// Frozen sky objects stay cached while state samples change at the recording cadence.
    fn load_state(
        &mut self,
        id: AssetId,
        state: EnvironmentState,
        cancelled: &impl Fn() -> bool,
    ) -> Result<FrozenEnvironment, ArchiveError> {
        let assets = match &self.assets {
            Some((current, assets)) if *current == id => Arc::clone(assets),
            _ => {
                let assets = Arc::new(sky_assets::load(&self.store, id, cancelled)?);
                self.assets = Some((id, Arc::clone(&assets)));
                assets
            }
        };
        if cancelled() {
            return Err(ArchiveError::Cancelled);
        }
        Ok(FrozenEnvironment {
            assets_identity: id,
            state,
            atmosphere: Arc::clone(&assets.atmosphere),
            skyboxes: assets.skyboxes.as_ref().map(Arc::clone),
        })
    }
}

/// Admits the small typed state while preserving checkpoint framing and rejecting unrelated records.
fn latest_state(
    record: ReplayRecord<'_>,
    latest: &mut Option<(AssetId, EnvironmentState)>,
) -> Result<(), ArchiveError> {
    match record.kind {
        RecordKind::CheckpointStart => *latest = None,
        RecordKind::CheckpointEnd => {}
        RecordKind::CheckpointPart | RecordKind::Scene => {
            *latest = Some(read_frame(&mut &record.payload[..])?)
        }
        _ => {
            return Err(ArchiveError::Invalid(
                "unexpected record in environment track",
            ));
        }
    }
    Ok(())
}

/// Keeps scene codec errors separate from live rendering or startup carrier failures.
fn io_error(_: std::io::Error) -> ArchiveError {
    ArchiveError::Invalid("truncated or unwritable environment record")
}

/// Asset-owner diagnostics remain available without coupling that owner to recording storage.
fn asset_error(error: assets::AssetError) -> ArchiveError {
    ArchiveError::Io {
        path: "recorded-environment-assets".into(),
        source: std::io::Error::other(error),
    }
}
