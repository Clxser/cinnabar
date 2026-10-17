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

pub const ENVIRONMENT_TRACK_SCHEMA: &str = "cinnabar.environment/2";
const LEGACY_TRACK_SCHEMA: &str = "cinnabar.environment/1";
pub const ENVIRONMENT_DIRECTORY: &str = "environment.cinreplay";
const FRAME_MAGIC: [u8; 8] = *b"CINEFR02";
const LEGACY_FRAME_MAGIC: [u8; 8] = *b"CINEFR01";
const MAX_ASSET_BYTES: u64 = 128 * 1024 * 1024;
type Skyboxes = [PackSkybox; SkyboxDimension::ALL.len()];

/// Boolean fields reject all encodings except the two defined wire values.
fn flag(reader: &mut Reader<'_>) -> Result<bool, ArchiveError> {
    match reader.u32()? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(ArchiveError::Invalid("invalid environment presence flag")),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnvironmentState {
    pub atmosphere: AtmosphereSnapshot,
    pub lightmap: LightmapSnapshot,
    pub fullbright: bool,
    pub animation_tick: u32,
    pub animation_partial: f32,
    /// Older recordings did not preserve dimension or camera shader settings.
    pub view: Option<RecordedEnvironmentView>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RecordedEnvironmentView {
    pub dimension: i32,
    pub enhanced: Option<RecordedEnhanced>,
    pub local_body_visible: bool,
}

/// Camera options are stored independently of Bevy and replayed without consulting live settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RecordedEnhanced {
    pub shadows: bool,
    pub shadow_resolution: u32,
    pub shadow_cascades: u32,
    pub shadow_distance: f32,
    pub bloom: bool,
    pub light_shafts: bool,
    pub waving: bool,
    pub water_reflections: bool,
}

impl EnvironmentState {
    /// Uses each owner's admission before a recorded value becomes a native draw input.
    fn valid(self) -> bool {
        AtmosphereSnapshot::from_words(self.atmosphere.words()).is_some()
            && LightmapSnapshot::from_words(self.lightmap.words()).is_some()
            && self.animation_partial.is_finite()
            && (0.0..1.0).contains(&self.animation_partial)
            && self.view.is_none_or(|view| {
                view.enhanced.is_none_or(|options| {
                    options.shadow_distance.is_finite() && options.shadow_distance > 0.0
                })
            })
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
    binary::u32(output, state.animation_partial.to_bits())?;
    binary::u32(output, u32::from(state.view.is_some()))?;
    if let Some(view) = state.view {
        binary::u32(output, view.dimension as u32)?;
        binary::u32(output, u32::from(view.local_body_visible))?;
        binary::u32(output, u32::from(view.enhanced.is_some()))?;
        if let Some(options) = view.enhanced {
            let flags = u32::from(options.shadows)
                | (u32::from(options.bloom) << 1)
                | (u32::from(options.light_shafts) << 2)
                | (u32::from(options.waving) << 3)
                | (u32::from(options.water_reflections) << 4);
            for value in [
                flags,
                options.shadow_resolution,
                options.shadow_cascades,
                options.shadow_distance.to_bits(),
            ] {
                binary::u32(output, value)?;
            }
        }
    }
    Ok(())
}

/// Fixed records use counted arrays so truncated, appended or incorrectly sized states are rejected.
fn read_frame(input: &mut dyn Read) -> Result<(AssetId, EnvironmentState), ArchiveError> {
    let mut reader = Reader::new(input);
    let magic = reader.bytes::<8>()?;
    if magic != FRAME_MAGIC && magic != LEGACY_FRAME_MAGIC {
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
    let mut state = EnvironmentState {
        atmosphere: atmosphere[0],
        lightmap: lightmap[0],
        fullbright,
        animation_tick: reader.u32()?,
        animation_partial: f32::from_bits(reader.u32()?),
        view: None,
    };
    if magic == FRAME_MAGIC && flag(&mut reader)? {
        let dimension = reader.u32()? as i32;
        let local_body_visible = flag(&mut reader)?;
        let enhanced = if flag(&mut reader)? {
            let flags = reader.u32()?;
            if flags & !31 != 0 {
                return Err(ArchiveError::Invalid("invalid recorded shader flags"));
            }
            Some(RecordedEnhanced {
                shadows: flags & 1 != 0,
                bloom: flags & 2 != 0,
                light_shafts: flags & 4 != 0,
                waving: flags & 8 != 0,
                water_reflections: flags & 16 != 0,
                shadow_resolution: reader.u32()?,
                shadow_cascades: reader.u32()?,
                shadow_distance: f32::from_bits(reader.u32()?),
            })
        } else {
            None
        };
        state.view = Some(RecordedEnvironmentView {
            dimension,
            enhanced,
            local_body_visible,
        });
    }
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
        if ![ENVIRONMENT_TRACK_SCHEMA, LEGACY_TRACK_SCHEMA]
            .contains(&reader.info().scene_schema.as_str())
        {
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
