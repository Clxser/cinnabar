//! Named inspection views are source-bound project metadata, independent of timeline camera shots.

use std::path::PathBuf;

use super::*;
use crate::{
    Project,
    error::limit,
    model::{MediaKind, MediaSource},
};

pub const MAX_SAVED_VIEW_NAME_BYTES: usize = 128;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CameraViewSource {
    pub path: PathBuf,
    pub sha256: Option<[u8; 32]>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedCameraView {
    pub id: Id,
    pub name: String,
    pub source: Id,
    pub source_identity: CameraViewSource,
    pub pose: CameraPose,
}

impl SavedCameraView {
    /// Captures exact camera values and the source identity available at the explicit save operation.
    pub fn new(
        id: Id,
        name: String,
        source: &MediaSource,
        pose: CameraPose,
    ) -> Result<Self, StudioError> {
        let view = Self {
            id,
            name,
            source: source.id,
            source_identity: CameraViewSource {
                path: source.path.clone(),
                sha256: source.sha256,
            },
            pose,
        };
        view.validate(source)?;
        Ok(view)
    }

    /// Unavailable bindings remain stored after relinking, but malformed names/poses/references are rejected.
    pub fn validate(&self, source: &MediaSource) -> Result<(), StudioError> {
        limit(
            "saved view name bytes",
            self.name.len(),
            MAX_SAVED_VIEW_NAME_BYTES,
        )?;
        if self.id.0 == 0 || self.name.trim().is_empty() || self.name.chars().any(char::is_control)
        {
            return Err(StudioError::Invalid(
                "saved camera views need a nonempty single-line name and nonzero identity",
            ));
        }
        if source.id.0 == 0 || source.id != self.source || source.kind != MediaKind::Replay {
            return Err(StudioError::Invalid(
                "saved camera views require their referenced replay source",
            ));
        }
        if self.source_identity.path.as_os_str().is_empty()
            || self.source_identity.path.as_os_str().len() > crate::model::MAX_MEDIA_PATH_UNITS
        {
            return Err(StudioError::Invalid(
                "saved camera view source path is invalid",
            ));
        }
        self.pose.validate()
    }

    /// Hash identity permits relocating known content; unhashed takes require the captured path.
    pub fn matches_source(&self, source: &MediaSource) -> bool {
        source.id == self.source
            && source.kind == MediaKind::Replay
            && match self.source_identity.sha256 {
                Some(hash) => source.sha256 == Some(hash),
                None => source.sha256.is_none() && source.path == self.source_identity.path,
            }
    }

    /// Recall checks both the active replay and the durable binding before exposing saved coordinates.
    pub fn recall(&self, project: &Project, active_source: Id) -> Result<CameraPose, StudioError> {
        let source = project
            .sources
            .iter()
            .find(|source| source.id == self.source)
            .ok_or(StudioError::Missing {
                kind: "saved camera view source",
                id: self.source.0,
            })?;
        self.validate(source)?;
        if active_source != self.source {
            return Err(StudioError::Invalid(
                "This view belongs to another take. Select that replay clip, or explicitly bind the view to the current take.",
            ));
        }
        if !self.matches_source(source) {
            return Err(StudioError::Invalid(
                "This view's recording changed. Bind to current take explicitly, or restore the original recording.",
            ));
        }
        Ok(self.pose)
    }

    /// Rebinding preserves every camera component and is applied only as an explicit undoable edit.
    pub fn rebound(&self, source: &MediaSource) -> Result<Self, StudioError> {
        Self::new(self.id, self.name.clone(), source, self.pose)
    }
}

#[cfg(test)]
mod tests;
