//! Cross-track publication succeeds as one source frame or invalidates all draft cursors together.

use std::path::Path;

use recording_studio::{
    MediaTime,
    archive::{ArchiveError, RecordingInfo},
    capture::{CameraRecording, RecordedView},
};

use super::{TerrainPatch, TerrainPlayback};
use crate::{
    actors::{ActorRecording, FrozenActors},
    environment::{EnvironmentRecording, FrozenEnvironment},
    journal::TerrainRecording,
};

pub struct SceneFrame {
    pub at: MediaTime,
    /// A seek changes interpolation, shadow, temporal antialiasing and audio history together.
    pub temporal_reset: bool,
    pub terrain: TerrainPatch,
    pub environment: FrozenEnvironment,
    pub actors: FrozenActors,
    pub view: RecordedView,
}

pub struct ScenePlayback {
    terrain: TerrainPlayback,
    environment: EnvironmentRecording,
    actors: ActorRecording,
    camera: CameraRecording,
    duration: MediaTime,
    position: Option<MediaTime>,
}

impl ScenePlayback {
    /// Opens independently versioned tracks from one take; absent scene coverage is an explicit error.
    /// This performs filesystem IO and belongs on a playback worker, never on the frame/input thread.
    pub fn open(parent: &Path, terrain_bytes: usize) -> Result<Self, ArchiveError> {
        let camera = CameraRecording::open(parent)?;
        let info = camera.info();
        let terrain = TerrainRecording::open(parent)?.into_playback(terrain_bytes);
        let environment = EnvironmentRecording::open(parent)?;
        let actors = ActorRecording::open(parent)?;
        if !same_take(info, terrain.info())
            || !same_take(info, environment.info())
            || !same_take(info, actors.info())
        {
            return Err(ArchiveError::Invalid(
                "native scene tracks belong to different recordings",
            ));
        }
        let duration = terrain
            .duration()
            .min(environment.duration())
            .min(actors.duration())
            .min(camera.duration());
        Ok(Self {
            terrain,
            environment,
            actors,
            camera,
            duration,
            position: None,
        })
    }

    /// Interrupted sources expose only the common committed interval of all required native tracks.
    pub const fn duration(&self) -> MediaTime {
        self.duration
    }

    /// Only completely admitted frames advance the coordinator's published source position.
    pub const fn position(&self) -> Option<MediaTime> {
        self.position
    }

    /// Builds one independent replacement frame from each track's nearest checkpoint.
    pub fn seek(
        &mut self,
        at: MediaTime,
        cancelled: impl Fn() -> bool,
    ) -> Result<SceneFrame, ArchiveError> {
        self.update(at, true, &cancelled)
    }

    /// Unchanged terrain is not rebuilt; environment and actor frames share immutable admitted objects.
    pub fn advance_to(
        &mut self,
        at: MediaTime,
        cancelled: impl Fn() -> bool,
    ) -> Result<SceneFrame, ArchiveError> {
        if self.position.is_none_or(|previous| at < previous) {
            return Err(ArchiveError::Invalid(
                "scene playback needs a seek before advancing",
            ));
        }
        self.update(at, false, &cancelled)
    }

    /// No partial terrain, weather or actor update can become the starting point for another frame.
    pub fn invalidate(&mut self) {
        self.terrain.invalidate();
        self.environment.invalidate();
        self.actors.invalidate();
        self.camera.invalidate();
        self.position = None;
    }

    /// All independently bounded source work finishes before a replacement/delta frame escapes.
    fn update(
        &mut self,
        at: MediaTime,
        seek: bool,
        cancelled: &impl Fn() -> bool,
    ) -> Result<SceneFrame, ArchiveError> {
        if at > self.duration {
            return Err(ArchiveError::Invalid(
                "scene position exceeds common track coverage",
            ));
        }
        let result = self.prepare(at, seek, cancelled);
        if result.is_ok() {
            self.position = Some(at);
        } else {
            self.invalidate();
        }
        result
    }

    /// Frames remain private staging until the final cancellation check after every track succeeds.
    fn prepare(
        &mut self,
        at: MediaTime,
        seek: bool,
        cancelled: &impl Fn() -> bool,
    ) -> Result<SceneFrame, ArchiveError> {
        let terrain = if seek {
            self.terrain.seek(at, cancelled)?
        } else {
            self.terrain.advance_to(at, cancelled)?
        };
        let environment = if seek {
            self.environment.restore(at, cancelled)?
        } else {
            self.environment.advance_to(at, cancelled)?.clone()
        };
        let actors = if seek {
            self.actors.restore(at, cancelled)?
        } else {
            self.actors.advance_to(at, cancelled)?.clone()
        };
        let view = if seek {
            self.camera.restore(at, cancelled)?
        } else {
            self.camera.advance_to(at, cancelled)?
        };
        if cancelled() {
            return Err(ArchiveError::Cancelled);
        }
        Ok(SceneFrame {
            at,
            temporal_reset: seek,
            terrain,
            environment,
            actors,
            view,
        })
    }
}

/// Exhaustive field matching forces source-identity admission to change when recording metadata grows.
fn same_take(primary: &RecordingInfo, track: &RecordingInfo) -> bool {
    let RecordingInfo {
        version,
        name,
        game_version,
        protocol_version,
        scene_schema: _,
        created_unix_ms,
        viewport,
        local_actor,
    } = primary;
    let RecordingInfo {
        version: other_version,
        name: other_name,
        game_version: other_game_version,
        protocol_version: other_protocol_version,
        scene_schema: _,
        created_unix_ms: other_created,
        viewport: other_viewport,
        local_actor: other_actor,
    } = track;
    version == other_version
        && name == other_name
        && game_version == other_game_version
        && protocol_version == other_protocol_version
        && created_unix_ms == other_created
        && viewport == other_viewport
        && local_actor == other_actor
}
