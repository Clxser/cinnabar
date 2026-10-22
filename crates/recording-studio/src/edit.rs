//! Transactional project editing and bounded undo/redo over immutable metadata snapshots.

use std::{collections::VecDeque, sync::Arc};

use serde::{Deserialize, Serialize};

use crate::{
    Id, MediaTime, PlaybackRate, Project, Span, StudioError,
    camera::CameraShot,
    effect::EffectClip,
    model::{AudioClip, Clip, ExportSettings, Framing, Marker, MediaSource, TitleClip, Transition},
};

mod operations;
#[cfg(test)]
mod save_tests;

pub const MAX_EDIT_COMMANDS: usize = 512;
pub const MAX_UNDO_STEPS: usize = 100;
pub const MAX_UNDO_BYTES: usize = 32 * 1024 * 1024;

/// Serializable intent is also the future agent boundary; it never exposes renderer or OS handles.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    SetTrackMuted {
        track: crate::tracks::Track,
        muted: bool,
    },
    Rename {
        name: String,
    },
    AddSource {
        source: MediaSource,
    },
    ReplaceSource {
        source: MediaSource,
    },
    AddClip {
        clip: Clip,
    },
    ReplaceClip {
        clip: Clip,
    },
    Delete {
        id: Id,
    },
    MoveClip {
        id: Id,
        at: MediaTime,
        track: u16,
    },
    TrimClip {
        id: Id,
        source: Span,
        ripple: bool,
    },
    SplitClip {
        id: Id,
        at: MediaTime,
        right: Id,
    },
    SetRate {
        id: Id,
        rate: PlaybackRate,
        ripple: bool,
    },
    SetSpeedRamp {
        id: Id,
        ramp: Option<crate::retime::SpeedRamp>,
        ripple: bool,
    },
    SetFraming {
        id: Id,
        framing: Framing,
    },
    AddShot {
        shot: CameraShot,
    },
    ReplaceShot {
        shot: CameraShot,
    },
    AddCameraView {
        view: crate::camera::SavedCameraView,
    },
    ReplaceCameraView {
        view: crate::camera::SavedCameraView,
    },
    DeleteCameraView {
        id: Id,
    },
    AddEffect {
        effect: EffectClip,
    },
    ReplaceEffect {
        effect: EffectClip,
    },
    AddAudio {
        audio: AudioClip,
    },
    ReplaceAudio {
        audio: AudioClip,
    },
    AddTitle {
        title: TitleClip,
    },
    ReplaceTitle {
        title: TitleClip,
    },
    AddMarker {
        marker: Marker,
    },
    /// Replaces a tempo grid atomically while preserving other marker kinds and out-of-range beats.
    SetBeatMarkers {
        span: Span,
        markers: Vec<Marker>,
    },
    ReplaceMarker {
        marker: Marker,
    },
    AddTransition {
        transition: Transition,
    },
    SetExport {
        settings: ExportSettings,
    },
    ClearAutomation {
        source: Option<Id>,
    },
}

#[derive(Clone)]
struct Revision {
    project: Arc<Project>,
    duration: MediaTime,
    bytes: usize,
    label: String,
}

/// A complete transaction either publishes a valid project or leaves project/history untouched.
pub struct Editor {
    current: Revision,
    undo: VecDeque<Revision>,
    redo: VecDeque<Revision>,
    revision: u64,
    saved_project: Arc<Project>,
    history_bytes: usize,
}

impl Editor {
    /// Opens admitted metadata. Source decoding and scene/archive caches are owned elsewhere.
    pub fn new(project: Project) -> Result<Self, StudioError> {
        let bytes = project.to_json()?.len();
        let duration = project.duration();
        let project = Arc::new(project);
        Ok(Self {
            saved_project: Arc::clone(&project),
            current: Revision {
                project,
                duration,
                bytes,
                label: String::new(),
            },
            undo: VecDeque::new(),
            redo: VecDeque::new(),
            revision: 1,
            history_bytes: 0,
        })
    }

    pub fn project(&self) -> &Arc<Project> {
        &self.current.project
    }

    /// Restoring metadata creates an unsaved copy with no false durable baseline or inherited undo history.
    pub fn recovered(project: Project) -> Result<Self, StudioError> {
        let mut editor = Self::new(project)?;
        editor.saved_project = Arc::new(Project::new(editor.project().name.clone()));
        Ok(editor)
    }

    /// Storage acknowledges the exact immutable snapshot, even if editing has advanced while saving.
    pub fn mark_saved_snapshot(&mut self, project: Arc<Project>) {
        self.saved_project = project;
    }
    /// Admission computes duration once; playback and undo use the matching immutable revision.
    pub const fn duration(&self) -> MediaTime {
        self.current.duration
    }

    pub const fn revision(&self) -> u64 {
        self.revision
    }
    pub fn dirty(&self) -> bool {
        !Arc::ptr_eq(&self.current.project, &self.saved_project)
    }
    pub fn undo_label(&self) -> Option<&str> {
        self.undo.back().map(|_| self.current.label.as_str())
    }
    pub fn redo_label(&self) -> Option<&str> {
        self.redo.back().map(|revision| revision.label.as_str())
    }

    /// Records a successful durable save only if its asynchronous revision is still current.
    pub fn mark_saved(&mut self, revision: u64) {
        if self.revision == revision {
            self.saved_project = Arc::clone(&self.current.project);
        }
    }

    /// Previews a batch against a private copy without changing the open project or its undo stack.
    pub fn draft(&self, commands: &[Command]) -> Result<Project, StudioError> {
        crate::error::limit("edit commands", commands.len(), MAX_EDIT_COMMANDS)?;
        let mut next = (*self.current.project).clone();
        for command in commands {
            operations::apply(&mut next, command)?;
        }
        crate::retime::anchors::synchronize(&self.current.project, &mut next)?;
        next.validate()?;
        Ok(next)
    }

    /// Commits an entire drag, automatic edit or multi-track operation as one reversible step.
    pub fn apply(
        &mut self,
        label: impl Into<String>,
        commands: &[Command],
    ) -> Result<bool, StudioError> {
        self.apply_guarded(label, commands, |_, _| Ok(()))
    }

    /// Workspace constraints inspect the fully synchronized draft before a single undo step is published.
    pub fn apply_guarded(
        &mut self,
        label: impl Into<String>,
        commands: &[Command],
        admit: impl FnOnce(&Project, &Project) -> Result<(), StudioError>,
    ) -> Result<bool, StudioError> {
        let label = label.into();
        crate::error::limit("edit label bytes", label.len(), 256)?;
        let next = self.draft(commands)?;
        admit(&self.current.project, &next)?;
        if next == *self.current.project {
            return Ok(false);
        }
        let bytes = next.to_json()?.len();
        let duration = next.duration();
        let revision = self
            .revision
            .checked_add(1)
            .ok_or(StudioError::IdentifiersExhausted)?;
        self.history_bytes -= self
            .redo
            .iter()
            .map(|revision| revision.bytes)
            .sum::<usize>();
        self.redo.clear();
        let previous = std::mem::replace(
            &mut self.current,
            Revision {
                project: Arc::new(next),
                duration,
                bytes,
                label,
            },
        );
        self.history_bytes += previous.bytes;
        self.undo.push_back(previous);
        self.revision = revision;
        self.trim_history();
        Ok(true)
    }

    /// Restores the previous complete transaction with no media copies or IO.
    pub fn undo(&mut self) -> Result<bool, StudioError> {
        let Some(previous) = self.undo.back() else {
            return Ok(false);
        };
        let bytes = previous.bytes;
        let revision = self
            .revision
            .checked_add(1)
            .ok_or(StudioError::IdentifiersExhausted)?;
        let previous = self.undo.pop_back().expect("the previous revision exists");
        let current = std::mem::replace(&mut self.current, previous);
        self.history_bytes = self.history_bytes - bytes + current.bytes;
        self.redo.push_back(current);
        self.revision = revision;
        self.trim_history();
        Ok(true)
    }

    /// Reapplies the exact snapshot previously undone rather than replaying side effects.
    pub fn redo(&mut self) -> Result<bool, StudioError> {
        let Some(next) = self.redo.back() else {
            return Ok(false);
        };
        let bytes = next.bytes;
        let revision = self
            .revision
            .checked_add(1)
            .ok_or(StudioError::IdentifiersExhausted)?;
        let next = self.redo.pop_back().expect("the redo revision exists");
        let current = std::mem::replace(&mut self.current, next);
        self.history_bytes = self.history_bytes - bytes + current.bytes;
        self.undo.push_back(current);
        self.revision = revision;
        self.trim_history();
        Ok(true)
    }

    /// Retains a fixed metadata budget, independently of source-media size and project duration.
    fn trim_history(&mut self) {
        while self.undo.len() + self.redo.len() > MAX_UNDO_STEPS
            || self.history_bytes > MAX_UNDO_BYTES
        {
            let oldest = self.undo.pop_front().or_else(|| self.redo.pop_front());
            let Some(oldest) = oldest else {
                break;
            };
            self.history_bytes -= oldest.bytes;
        }
    }
}
