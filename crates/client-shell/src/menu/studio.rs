//! Native studio state, metadata editing and bounded filesystem job servicing.

use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

use launcher::menu::studio::{Action, DiscardIntent, Snapshot};
use recording_studio::{
    Id, MediaTime, PlaybackRate, Project, Span,
    archive::RecordingReader,
    edit::{Command, Editor},
    model::{Clip, Framing, MediaKind, MediaSource, TitleClip, TitleStyle},
    playback::Transport,
    storage::{LibraryKind, ProjectStorage, StorageRequest, StorageResult, StorageValue},
};

mod audio;
mod bridge;
mod camera;
mod clips;
mod creative;
mod diagnostics;
mod effects;
mod export_range;
mod library;
mod media;
mod monitoring;
mod persistence;
mod recovery;
mod scene;
mod settings;
mod speed;
pub(super) mod timeline;
mod titles;
mod workspace;

pub(super) struct Studio {
    pub view: Arc<Snapshot>,
    root: PathBuf,
    storage: Option<ProjectStorage>,
    editor: Option<Editor>,
    transport: Transport,
    audio_pause: audio::PauseSync,
    library_job: Option<u64>,
    open_job: Option<u64>,
    save_job: Option<persistence::SaveJob>,
    recovery: recovery::Recovery,
    diagnostics: diagnostics::Diagnostics,
    document: u64,
    replays: BTreeMap<Id, Arc<RecordingReader>>,
    scrub: Option<(i32, u32)>,
    gesture: Option<timeline::Gesture>,
    capture_request: Option<recording_studio::capture::CaptureRequest>,
    export_request: Option<recording_studio::export::ExportRequest>,
    export_cancel: bool,
    pub(super) title_text: ui::ChatEditor,
    title_edit: Option<(u64, Id, String)>,
    media_job: Option<media::MediaJob>,
    creative: creative::CreativeTasks,
    preferences_job: Option<u64>,
    preferences_changed: bool,
    thumbnails_job: Option<u64>,
    thumbnails_cancel: recording_studio::export::Cancellation,
    open_cancel: recording_studio::export::Cancellation,
    recording_recovery: Option<recording_studio::storage::RecordingRecovery>,
    target_coverage: Option<recording_studio::camera::coverage::TargetCoverageReader>,
    target_verifier: Option<recording_studio::camera::coverage::TargetVerifier>,
    pub(super) destination_text: ui::ChatEditor,
    pub(super) camera_text: ui::ChatEditor,
    camera_edit: Option<camera::CameraValueEdit>,
    pub(super) camera_view_text: ui::ChatEditor,
    pub(super) subject_text: ui::ChatEditor,
    camera_view_edit: Option<camera::CameraViewEdit>,
    camera_interaction: camera::CameraInteraction,
    pub(super) clip_text: ui::ChatEditor,
    clip_edit: Option<clips::ClipTimeEdit>,
    pub(super) range_text: ui::ChatEditor,
    range_edit: Option<export_range::RangeEdit>,
    pub(super) search_text: ui::ChatEditor,
    pub(super) seek_text: ui::ChatEditor,
    seek_edit: Option<(u64, u64)>,
}

impl std::fmt::Debug for Studio {
    /// Reports compact control state without traversing projects, recorded assets or worker queues.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Studio")
            .field("document", &self.document)
            .field("has_project", &self.editor.is_some())
            .field("position", &self.transport.position())
            .field("loading", &self.view.loading)
            .field("capture", &self.view.capture.phase)
            .field("export", &self.view.export.phase)
            .finish_non_exhaustive()
    }
}

impl Studio {
    /// Defers worker startup and filesystem access until the studio is actually opened.
    pub fn new(root: PathBuf) -> Self {
        Self {
            view: Arc::new(Snapshot {
                show_library: true,
                visible_duration: MediaTime(30_000_000),
                ..Snapshot::default()
            }),
            root,
            storage: None,
            editor: None,
            transport: Transport::default(),
            audio_pause: Default::default(),
            library_job: None,
            open_job: None,
            save_job: None,
            recovery: Default::default(),
            diagnostics: Default::default(),
            document: 0,
            replays: BTreeMap::new(),
            scrub: None,
            gesture: None,
            capture_request: None,
            export_request: None,
            export_cancel: false,
            title_text: super::input::fields::field_editor(launcher::menu::MenuField::StudioTitle),
            title_edit: None,
            media_job: None,
            creative: Default::default(),
            preferences_job: None,
            preferences_changed: false,
            thumbnails_job: None,
            thumbnails_cancel: Default::default(),
            open_cancel: Default::default(),
            recording_recovery: None,
            target_coverage: None,
            target_verifier: None,
            destination_text: super::input::fields::field_editor(
                launcher::menu::MenuField::StudioDestination,
            ),
            camera_text: super::input::fields::field_editor(
                launcher::menu::MenuField::StudioCamera(
                    recording_studio::camera::CameraParameter::X,
                ),
            ),
            camera_edit: None,
            camera_view_text: super::input::fields::field_editor(
                launcher::menu::MenuField::StudioCameraViewName(None),
            ),
            camera_view_edit: None,
            subject_text: super::input::fields::field_editor(
                launcher::menu::MenuField::StudioSubjectSearch,
            ),
            camera_interaction: Default::default(),
            clip_text: super::input::fields::field_editor(launcher::menu::MenuField::StudioClip(
                launcher::menu::studio::ClipTimeField::Position,
            )),
            clip_edit: None,
            range_text: super::input::fields::field_editor(
                launcher::menu::MenuField::StudioExportRange(
                    launcher::menu::studio::ExportRangeField::In,
                ),
            ),
            range_edit: None,
            search_text: super::input::fields::field_editor(
                launcher::menu::MenuField::StudioLibrarySearch,
            ),
            seek_text: super::input::fields::field_editor(launcher::menu::MenuField::StudioSeek),
            seek_edit: None,
        }
    }

    /// Opens the existing document or library without resetting unsaved edits.
    pub fn open(&mut self) {
        self.scrub = None;
        self.scan_recovery();
        if let Err(error) = self.ensure_storage().and_then(|()| self.refresh()) {
            Arc::make_mut(&mut self.view).error = Some(error);
        }
        if self.preferences_job.is_none() && !self.preferences_changed {
            match self.storage.as_mut().map(|storage| {
                storage.request(StorageRequest::LoadCaptureSettings {
                    root: self.root.clone(),
                })
            }) {
                Some(Ok(job)) => self.preferences_job = Some(job),
                Some(Err(error)) => Arc::make_mut(&mut self.view).error = Some(error.to_string()),
                None => {}
            }
        }
    }

    /// Dispatches only native UI intent. Editing never changes the live session or recorded media.
    pub fn activate(&mut self, action: Action) {
        if self.view.workflow.recovery.busy
            && !matches!(
                action,
                Action::CancelOpen
                    | Action::CancelExport
                    | Action::ToggleRecording
                    | Action::Bookmark
                    | Action::ClearError
            )
        {
            return;
        }
        if self.view.workflow.recovery.visible
            && !matches!(
                action,
                Action::ShowRecovery
                    | Action::HideRecovery
                    | Action::ScanRecovery
                    | Action::RestoreRecovery(_)
                    | Action::DiscardRecovery(_)
                    | Action::RetryAutosave
                    | Action::ConfirmDiscard
                    | Action::CancelDiscard
                    | Action::CancelOpen
                    | Action::CancelExport
                    | Action::ToggleRecording
                    | Action::Bookmark
                    | Action::ClearError
            )
        {
            return;
        }
        if self.view.loading
            && !matches!(
                action,
                Action::CancelOpen
                    | Action::CancelExport
                    | Action::ClearError
                    | Action::ToggleRecording
                    | Action::Bookmark
            )
        {
            return;
        }
        Arc::make_mut(&mut self.view).error = None;
        if let Err(error) = self.apply_action(action) {
            Arc::make_mut(&mut self.view).error = Some(error);
        }
        self.publish_editor();
    }

    /// Services at most the storage queue's bounded completions and advances a caller-owned clock.
    pub fn tick(&mut self, elapsed: f64, visible: bool) {
        self.poll_media();
        self.poll_creative();
        if visible {
            self.prime_recovery_scan();
        }
        self.tick_recovery(elapsed);
        self.poll_diagnostics();
        if self.storage.is_none() {
            return;
        }
        for _ in 0..4 {
            let result = self.storage.as_ref().expect("storage exists").poll();
            match result {
                Ok(Some(result)) => self.completed(result),
                Ok(None) => break,
                Err(error) => {
                    let view = Arc::make_mut(&mut self.view);
                    view.error = Some(error.to_string());
                    view.library_loading = false;
                    view.loading = false;
                    view.saving = false;
                    self.storage = None;
                    self.library_job = None;
                    self.open_job = None;
                    self.save_job = None;
                    self.preferences_job = None;
                    self.thumbnails_job = None;
                    self.warn_retained_recovery();
                    break;
                }
            }
        }
        if visible
            && !self.view.show_library
            && self.transport.playing()
            && let Ok(elapsed) = MediaTime::from_seconds(elapsed)
            && let Some(editor) = &self.editor
            && let Some(available) = self.view.preview.playback_ceiling(
                [self.document, editor.revision(), self.transport.epoch()],
                editor.project().export.fps,
            )
        {
            self.advance_playback(elapsed, available);
        }
        if self.view.position != self.transport.position()
            || self.view.playing != self.transport.playing()
        {
            let view = Arc::make_mut(&mut self.view);
            view.position = self.transport.position();
            view.playing = self.transport.playing();
        }
        if visible
            && self.transport.playing()
            && self.view.workspace.follow_playhead
            && (self.transport.position() < self.view.visible_start
                || self.transport.position().0
                    >= self
                        .view
                        .visible_start
                        .0
                        .saturating_add(self.view.visible_duration.0 * 9 / 10))
        {
            self.reveal_playhead();
        }
    }

    /// Lists actual current controls for keyboard/gamepad fallback before painted focus is available.
    pub fn focus_actions(&self) -> Vec<launcher::menu::MenuAction> {
        use launcher::menu::MenuAction;
        let mut actions = if self.view.workflow.recovery.busy {
            if self.view.loading {
                vec![Action::CancelOpen]
            } else {
                Vec::new()
            }
        } else if self.view.discard.is_some() {
            vec![Action::CancelDiscard, Action::ConfirmDiscard]
        } else if self.view.loading {
            let mut actions = vec![Action::CancelOpen];
            if self.view.capture.phase.busy() {
                actions.push(Action::ToggleRecording);
            }
            actions
        } else if self.view.workflow.recovery.visible {
            let mut actions = vec![Action::HideRecovery, Action::ScanRecovery];
            if !self.view.workflow.recovery.loading {
                for (index, entry) in self.view.workflow.recovery.entries.iter().enumerate() {
                    if entry.recoverable {
                        actions.push(Action::RestoreRecovery(index));
                    }
                    actions.push(Action::DiscardRecovery(index));
                }
            }
            actions
        } else if self.view.workflow.show_settings {
            vec![
                Action::Library,
                Action::Save,
                if self.view.export.phase.busy() {
                    Action::CancelExport
                } else {
                    Action::Export
                },
            ]
        } else if self.view.show_library || self.editor.is_none() {
            let mut actions = vec![Action::NewProject, Action::Refresh];
            actions.extend(self.library_focus_actions());
            actions.extend((0..self.view.library.len()).map(|index| Action::Open {
                generation: self.view.workspace.library.generation,
                index,
            }));
            actions
        } else {
            vec![
                Action::Library,
                Action::Save,
                if self.view.export.phase.busy() {
                    Action::CancelExport
                } else {
                    Action::Export
                },
                Action::Undo,
                Action::Redo,
                Action::TogglePlayback,
                Action::Step(-1),
                Action::Step(1),
                Action::Split,
                Action::DeleteSelected,
                Action::AddTitle,
                Action::UseNativeReplay,
            ]
        };
        if self.view.discard.is_none()
            && !self.view.loading
            && !self.view.show_library
            && !self.view.workflow.show_settings
            && self.editor.is_some()
            && !self.view.workflow.recovery.visible
            && !self.view.workflow.recovery.busy
        {
            actions.extend(self.workspace_focus_actions());
            actions.extend(self.monitoring_focus_actions());
            actions.extend(self.camera_focus_actions());
            actions.extend(self.audio_focus_actions());
            actions.extend(self.clip_focus_actions());
            actions.extend(self.speed_focus_actions());
            actions.extend(self.effect_focus_actions());
            actions.extend(self.creative_focus_actions());
            actions.extend(self.title_focus_actions());
        }
        if self.view.discard.is_none()
            && !self.view.loading
            && !self.view.workflow.recovery.visible
            && !self.view.workflow.recovery.busy
        {
            actions.extend(self.workflow_focus_actions());
        }
        let mut actions: Vec<_> = actions.into_iter().map(MenuAction::Studio).collect();
        if self.view.export.phase.busy()
            && !actions.contains(&MenuAction::Studio(Action::CancelExport))
        {
            actions.push(MenuAction::Studio(Action::CancelExport));
        }
        if self.view.discard.is_none() && !self.view.loading {
            actions.push(MenuAction::Studio(Action::ToggleRecording));
            if self.view.capture.phase == recording_studio::capture::CapturePhase::Recording {
                actions.push(MenuAction::Studio(Action::Bookmark));
            }
        }
        actions
    }

    /// Applies one action and returns a user-visible error instead of swallowing stale selections.
    fn apply_action(&mut self, action: Action) -> Result<(), String> {
        if matches!(
            action,
            Action::Seek(_)
                | Action::Step(_)
                | Action::Scrub { .. }
                | Action::TogglePlayback
                | Action::Undo
                | Action::Redo
                | Action::Library
        ) {
            self.cancel_camera_value();
            self.cancel_clip_time();
            self.cancel_monitor_seek();
            Arc::make_mut(&mut self.view).speed_key = None;
        }
        match action {
            Action::Creative(action) => self.creative_action(action)?,
            Action::Monitoring(action) => self.monitoring_action(action)?,
            Action::Workspace(action) => self.workspace_action(action),
            Action::ScanRecovery => self.scan_recovery(),
            Action::ShowRecovery => self.show_recovery(),
            Action::HideRecovery => Arc::make_mut(&mut self.view).workflow.recovery.visible = false,
            Action::RestoreRecovery(index) => {
                let intent = self.recovery_restore_intent(index)?;
                self.leave(intent, false)?;
            }
            Action::DiscardRecovery(index) => self.discard_recovery(index)?,
            Action::RetryAutosave => self.retry_autosave(),
            Action::CheckDependencies => self.check_dependencies()?,
            Action::FullExportRange => self.full_export_range()?,
            Action::SetExportRangeAtPlayhead(field) => self.export_range_at_playhead(field)?,
            Action::EditExportRange(field) => self.begin_export_range(field)?,
            Action::ApplyExportRange => self.apply_export_range()?,
            Action::CancelExportRange => self.cancel_export_range(),
            Action::UseCpuEncoder => self.use_cpu_encoder()?,
            Action::Timeline(action) => self.timeline_action(action)?,
            Action::ImportMedia => self.begin_media(false)?,
            Action::RelinkMedia => self.begin_media(true)?,
            Action::CancelMedia => self.cancel_media(),
            Action::ToggleSettings => {
                if !self.view.workflow.show_settings {
                    self.transport.pause();
                    self.release_camera();
                }
                let workflow = &mut Arc::make_mut(&mut self.view).workflow;
                workflow.show_settings = !workflow.show_settings;
            }
            Action::CycleCaptureMode => self.cycle_capture_mode()?,
            Action::CycleCaptureFps => self.cycle_capture_fps()?,
            Action::CycleCaptureSize => self.cycle_capture_size()?,
            Action::ToggleCaptureHand => self.toggle_capture_hand()?,
            Action::ToggleCaptureHud => self.toggle_capture_hud()?,
            Action::CycleExportFps
            | Action::CycleExportSize
            | Action::CycleExportCodec
            | Action::CycleExportEncoder
            | Action::AdjustExportQuality(_)
            | Action::ToggleExportAudio
            | Action::ToggleRecordedHud => self.output_setting(action)?,
            Action::EditExportDestination => self.begin_destination(),
            Action::ApplyExportDestination => self.apply_destination()?,
            Action::CancelExportDestination => {
                Arc::make_mut(&mut self.view).workflow.destination_draft = None
            }
            Action::Export => {
                self.cancel_diagnostics();
                self.queue_export()?;
            }
            Action::CancelExport => self.export_cancel = true,
            Action::ToggleRecording => {
                self.cancel_diagnostics();
                self.queue_capture_toggle();
            }
            Action::Bookmark => {
                if self.capture_request.is_none()
                    && self.view.capture.phase == recording_studio::capture::CapturePhase::Recording
                {
                    self.capture_request =
                        Some(recording_studio::capture::CaptureRequest::Bookmark);
                }
            }
            Action::NewProject => self.leave(DiscardIntent::NewProject, false)?,
            Action::Open { generation, index } => {
                let intent = self.library_intent(generation, index)?;
                self.leave(intent, false)?;
            }
            Action::LibraryTools(action) => self.library_action(action)?,
            Action::ConfirmDiscard => {
                if let Some(intent) = self.view.discard.clone() {
                    self.leave(intent, true)?;
                }
            }
            Action::CancelDiscard => Arc::make_mut(&mut self.view).discard = None,
            Action::CancelOpen => {
                if self.view.loading && self.view.workflow.recovery.busy {
                    self.cancel_recovery_restore();
                }
                self.open_cancel.cancel();
                self.open_job = None;
                Arc::make_mut(&mut self.view).loading = false;
                self.warn_retained_recovery();
            }
            Action::Refresh => {
                self.ensure_storage()?;
                self.refresh()?;
            }
            Action::Library => {
                self.transport.pause();
                self.open_job = None;
                let view = Arc::make_mut(&mut self.view);
                view.loading = false;
                view.show_library = !view.show_library || self.editor.is_none();
                if self.view.show_library {
                    self.refresh()?;
                }
            }
            Action::Save => self.save()?,
            Action::Undo => {
                if let Some(editor) = &mut self.editor {
                    editor.undo().map_err(|e| e.to_string())?;
                }
            }
            Action::Redo => {
                if let Some(editor) = &mut self.editor {
                    editor.redo().map_err(|e| e.to_string())?;
                }
            }
            Action::TogglePlayback => {
                if self.transport.playing() {
                    self.transport.pause();
                } else if let Some(editor) = &self.editor {
                    self.transport.play(editor.duration());
                }
            }
            Action::Step(frames) => {
                let project = self.project()?;
                let (fps, duration) = (project.export.fps, project.duration());
                self.transport
                    .step(i64::from(frames), fps, duration)
                    .map_err(|e| e.to_string())?;
            }
            Action::Seek(time) => {
                let project = self.project()?;
                let (fps, duration) = (project.export.fps, project.duration());
                self.transport
                    .seek(fps.snap(time).map_err(|e| e.to_string())?, duration);
            }
            Action::Scrub { .. } | Action::TimelineOverview { .. } => {}
            Action::DragItem { id, .. } => self.select_timeline_item(id)?,
            Action::Panel(panel) => Arc::make_mut(&mut self.view).workspace.panel = panel,
            Action::ToggleRipple => {
                let view = Arc::make_mut(&mut self.view);
                view.workspace.ripple = !view.workspace.ripple;
            }
            Action::ToggleFollowPlayhead => {
                let view = Arc::make_mut(&mut self.view);
                view.workspace.follow_playhead = !view.workspace.follow_playhead;
            }
            Action::FitTimeline => self.fit_timeline(),
            Action::RevealPlayhead => self.reveal_playhead(),
            Action::Select(id) => {
                self.select_timeline_item(id)?;
                if self.project()?.clips.iter().any(|clip| clip.id == id)
                    || self.project()?.effects.iter().any(|item| item.id == id)
                    || self.project()?.shots.iter().any(|item| item.id == id)
                    || self.project()?.titles.iter().any(|item| item.id == id)
                {
                    Arc::make_mut(&mut self.view).workspace.panel =
                        launcher::menu::studio::EditorPanel::Inspector;
                    Arc::make_mut(&mut self.view).workspace.inspector_hidden = false;
                }
            }
            Action::Split => {
                let project = self.project()?;
                let id = self
                    .view
                    .selected
                    .ok_or("Select a replay or video clip to split.")?;
                let right = project.next_id().map_err(|e| e.to_string())?;
                self.edit(
                    "Split clip",
                    &[Command::SplitClip {
                        id,
                        at: self.transport.position(),
                        right,
                    }],
                )?;
            }
            Action::DeleteSelected => self.delete_timeline_selection()?,
            Action::SetRate(rate) => {
                let id = self
                    .view
                    .selected
                    .ok_or("Select a replay or video clip to change its speed.")?;
                self.edit(
                    "Change speed",
                    &[Command::SetRate {
                        id,
                        rate,
                        ripple: self.view.workspace.ripple,
                    }],
                )?;
            }
            Action::EditClipTime(field) => self.begin_clip_time(field)?,
            Action::ApplyClipTime => self.apply_clip_time()?,
            Action::CancelClipTime => self.cancel_clip_time(),
            Action::AdjustFraming {
                parameter,
                direction,
            } => self.adjust_framing(parameter, direction)?,
            Action::ResetFraming => self.reset_framing()?,
            Action::MoveClipLayer(direction) => self.move_clip_layer(direction)?,
            Action::TransitionToNext(kind) => self.transition_to_next(kind)?,
            Action::RemoveClipTransition => self.remove_clip_transition()?,
            Action::AddTitle => {
                let project = self.project()?;
                let id = project.next_id().map_err(|e| e.to_string())?;
                let start = self.transport.position();
                let end = start
                    .checked_add(MediaTime(3_000_000))
                    .map_err(|e| e.to_string())?;
                let title = TitleClip {
                    id,
                    span: Span::new(start, end).map_err(|e| e.to_string())?,
                    text: project.name.clone(),
                    anchor: [0.5, 0.8],
                    size: 0.08,
                    color: [1.0; 4],
                    style: TitleStyle::Heading,
                    shadow: true,
                    fade_in: MediaTime(200_000),
                    fade_out: MediaTime(200_000),
                };
                self.edit("Add title", &[Command::AddTitle { title }])?;
                Arc::make_mut(&mut self.view).selected = Some(id);
            }
            Action::AddEffect(kind) => self.add_effect(kind)?,
            Action::AddLook(look) => self.add_look(look)?,
            Action::SetSpeedKey { rate, ripple } => self.set_speed_key(rate, ripple)?,
            Action::DeleteSpeedKey { ripple } => self.delete_speed_key(ripple)?,
            Action::CycleSpeedEasing { ripple } => self.cycle_speed_easing(ripple)?,
            Action::ClearSpeedRamp { ripple } => self.clear_speed_ramp(ripple)?,
            Action::SelectSpeedKey { clip, at } => self.select_speed_key(clip, at)?,
            Action::StepSpeedKey(direction) => self.step_speed_key(direction)?,
            Action::EditTitleText => self.begin_title_text()?,
            Action::ApplyTitleText => self.apply_title_text()?,
            Action::CancelTitleText => self.cancel_title_text(),
            Action::CycleTitleStyle => self.cycle_title_style()?,
            Action::ToggleTitleShadow => self.toggle_title_shadow()?,
            Action::AdjustTitle {
                parameter,
                direction,
            } => self.adjust_title(parameter, direction)?,
            Action::TrimTitleStart => self.trim_title(true)?,
            Action::TrimTitleEnd => self.trim_title(false)?,
            Action::MoveTitle(frames) => self.move_title(frames)?,
            Action::ToggleEffect => self.toggle_effect()?,
            Action::ToggleAudioMute => self.toggle_audio_mute()?,
            Action::AdjustAudio {
                parameter,
                direction,
            } => self.adjust_audio(parameter, direction)?,
            Action::AdjustEffect(direction) => self.adjust_effect(direction)?,
            Action::AddEffectKey => self.key_effect()?,
            Action::DeleteEffectKey => self.delete_effect_key()?,
            Action::CycleEffectEasing => self.cycle_effect_easing()?,
            Action::StepEffectKey(direction) => self.step_effect_key(direction)?,
            Action::SelectEffectKey { effect, at } => self.select_effect_key(effect, at)?,
            Action::MoveEffect(frames) => self.move_effect(frames)?,
            Action::UseNativeReplay => self.use_native_replay()?,
            Action::CameraViewport { .. } => self.focus_camera()?,
            Action::CameraAxisControl { .. } | Action::CameraRotationControl { .. } => {}
            Action::SetCameraDragAxis(axis) => {
                Arc::make_mut(&mut self.view).camera_workspace.drag_axis = axis;
            }
            Action::CycleCameraSnap => {
                let workspace = &mut Arc::make_mut(&mut self.view).camera_workspace;
                workspace.snap = workspace.snap.next();
            }
            Action::CycleCameraGuides => {
                let workspace = &mut Arc::make_mut(&mut self.view).camera_workspace;
                workspace.guides = workspace.guides.next();
            }
            Action::FrameCameraSubject => self.frame_camera_subject()?,
            Action::CameraViews(action) => self.camera_views_action(action)?,
            Action::CameraNavigation(action) => self.camera_navigation_action(action)?,
            Action::CameraBlend(action) => self.camera_blend_action(action)?,
            Action::CameraSubjects(action) => self.camera_subjects_action(action)?,
            Action::CameraControl { shot, at, .. } => self.select_camera_key(shot, at)?,
            Action::CameraSubject(reference) => self.select_camera_subject(reference)?,
            Action::ReleaseCamera => self.release_camera(),
            Action::ApplyCameraView => self.apply_camera_view()?,
            Action::ResetCameraView => self.reset_camera_view(),
            Action::ResetCameraHandles => self.reset_camera_handles()?,
            Action::CreateCamera(kind) => self.create_camera(kind)?,
            Action::CycleCameraTarget(direction) => self.cycle_camera_target(direction),
            Action::ToggleCameraSubjectFilter => self.toggle_camera_subject_filter(),
            Action::RetargetCamera => self.retarget_camera()?,
            Action::AdjustCamera {
                parameter,
                direction,
            } => self.adjust_camera(parameter, direction)?,
            Action::AddCameraKey => self.add_camera_key()?,
            Action::DeleteCameraKey => self.delete_camera_key()?,
            Action::AddCameraFovKey
            | Action::DeleteCameraFovKey
            | Action::ClearCameraFov
            | Action::CycleCameraFovEasing
            | Action::StepCameraFovKey(_) => self.camera_fov_action(action)?,
            Action::SelectCameraFovKey { shot, at } => self.select_camera_fov_key(shot, at)?,
            Action::CycleCameraEasing => self.cycle_camera_easing()?,
            Action::ToggleCameraSmoothing => self.toggle_camera_smoothing()?,
            Action::ToggleCameraDistanceTiming => self.toggle_camera_distance_timing()?,
            Action::CycleCameraGlobalEasing => self.cycle_camera_global_easing()?,
            Action::ToggleCameraContinuity => self.toggle_camera_continuity()?,
            Action::StepCameraKey(direction) => self.step_camera_key(direction)?,
            Action::SelectCameraKey { shot, at } => self.select_camera_key(shot, at)?,
            Action::MoveCameraKey(direction) => self.move_camera_key(direction)?,
            Action::EditCameraValue(parameter) => self.begin_camera_value(parameter)?,
            Action::ApplyCameraValue => self.apply_camera_value()?,
            Action::CancelCameraValue => self.cancel_camera_value(),
            Action::TrimCameraStart => self.trim_camera(true)?,
            Action::TrimCameraEnd => self.trim_camera(false)?,
            Action::ToggleCameraFine => {
                let view = Arc::make_mut(&mut self.view);
                view.camera_fine = !view.camera_fine;
            }
            Action::Zoom(direction) => {
                self.zoom_timeline(f64::from(direction), 0.5);
            }
            Action::Pan(direction) => {
                self.pan_timeline(f64::from(direction) * 0.5);
            }
            Action::ClearError => {}
        }
        Ok(())
    }

    /// Guards document replacement while allowing a dirty editor to stay open behind the library.
    fn leave(&mut self, intent: DiscardIntent, confirmed: bool) -> Result<(), String> {
        if !confirmed && self.editor.as_ref().is_some_and(Editor::dirty) {
            Arc::make_mut(&mut self.view).discard = Some(intent);
            return Ok(());
        }
        if confirmed && !self.recovery_leave(intent.clone())? {
            return Ok(());
        }
        match intent {
            DiscardIntent::RestoreRecovery(id) => self.begin_restore(id)?,
            DiscardIntent::NewProject => {
                let editor =
                    Editor::new(Project::new("Untitled project")).map_err(|e| e.to_string())?;
                self.install(editor, None);
            }
            DiscardIntent::Open {
                path,
                kind,
                interrupted,
            } => self.open_library_item(path, kind, interrupted)?,
        }
        Ok(())
    }

    /// Adds a recording as immutable source media; a recovered recording uses only its committed duration.
    fn import_replay(&mut self, path: PathBuf, reader: Arc<RecordingReader>) -> Result<(), String> {
        let duration = reader.duration();
        if duration == MediaTime::ZERO {
            return Err("This recording has no playable frames after its initial snapshot.".into());
        }
        let source = Id(1);
        let mut editor =
            Editor::new(Project::new(&reader.info().name)).map_err(|e| e.to_string())?;
        editor
            .apply(
                "Add replay",
                &[
                    Command::AddSource {
                        source: MediaSource {
                            id: source,
                            name: reader.info().name.clone(),
                            path,
                            kind: MediaKind::Replay,
                            duration,
                            size: reader.info().viewport,
                            frame_rate: None,
                            sha256: None,
                        },
                    },
                    Command::AddClip {
                        clip: Clip {
                            id: Id(2),
                            source,
                            source_range: Span::new(MediaTime::ZERO, duration)
                                .map_err(|e| e.to_string())?,
                            at: MediaTime::ZERO,
                            rate: PlaybackRate::default(),
                            speed_ramp: None,
                            track: 0,
                            framing: Framing::default(),
                            enabled: true,
                        },
                    },
                ],
            )
            .map_err(|e| e.to_string())?;
        self.install(editor, None);
        self.replays.insert(source, reader);
        Ok(())
    }

    /// Replaces the active document only after the selected input has been fully admitted.
    fn install(&mut self, editor: Editor, path: Option<PathBuf>) {
        self.creative = Default::default();
        self.reset_camera_view();
        Arc::make_mut(&mut self.view).camera_workspace = Default::default();
        if let Some(job) = &self.media_job {
            job.cancellation.cancel();
        }
        self.cancel_title_text();
        self.cancel_camera_value();
        self.cancel_camera_view_name();
        self.cancel_clip_time();
        self.cancel_export_range();
        self.document = self.document.wrapping_add(1);
        self.reset_recovery_document();
        self.editor = Some(editor);
        self.replays.clear();
        self.transport = Transport::default();
        let view = Arc::make_mut(&mut self.view);
        view.loading = false;
        view.file = path;
        view.show_library = false;
        view.selected = None;
        view.workspace.timeline = Default::default();
        view.workspace.media_hidden = true;
        view.workspace.inspector_hidden = true;
        view.workspace.property_selection = None;
        view.workflow.creative = Default::default();
        view.camera_target = None;
        view.speed_key = None;
        view.preview = Default::default();
        view.visible_start = MediaTime::ZERO;
        view.discard = None;
        view.notice = None;
        self.publish_editor();
    }

    /// Keeps per-frame presentation cheap: projects and library arrays remain shared publications.
    fn publish_editor(&mut self) {
        self.reconcile_creative();
        self.publish_monitoring();
        self.reconcile_timeline_selection();
        self.reconcile_inspector_selection();
        self.reconcile_clip_time();
        self.reconcile_export_range();
        self.reconcile_camera_workspace();
        let view = Arc::make_mut(&mut self.view);
        if let Some(editor) = &self.editor {
            if view
                .project
                .as_ref()
                .is_none_or(|project| !Arc::ptr_eq(project, editor.project()))
            {
                match recording_studio::lanes::TimelineLanes::new(editor.project()) {
                    Ok(lanes) => view.lanes = Arc::new(lanes),
                    Err(error) => {
                        view.lanes = Arc::default();
                        view.error = Some(error.to_string());
                    }
                }
            }
            view.project = Some(Arc::clone(editor.project()));
            view.dirty = editor.dirty();
            view.undo = editor.undo_label().map(str::to_owned);
            view.redo = editor.redo_label().map(str::to_owned);
        }
        view.position = self.transport.position();
        view.playing = self.transport.playing();
    }

    /// Reads the current admitted project without cloning its tracks.
    fn project(&self) -> Result<&Arc<Project>, String> {
        self.editor
            .as_ref()
            .map(Editor::project)
            .ok_or_else(|| "Create or open a project first.".into())
    }

    /// Commits one reversible metadata transaction and keeps the playhead inside its new duration.
    fn edit(&mut self, label: &str, commands: &[Command]) -> Result<(), String> {
        let editor = self
            .editor
            .as_mut()
            .ok_or("Create or open a project first.")?;
        let locked = &self.view.workspace.timeline.locked;
        editor
            .apply_guarded(label, commands, |before, after| {
                recording_studio::timeline::ensure_unlocked(before, after, locked)
            })
            .map_err(|e| e.to_string())?;
        let duration = editor.duration();
        self.cancel_camera_value();
        self.cancel_clip_time();
        if self.transport.position() > duration {
            self.transport.seek(duration, duration);
        }
        Ok(())
    }

    /// A captured drag continues outside the ruler until release and always seeks to an output frame.
    pub fn pointer(
        &mut self,
        hovered: Option<launcher::menu::MenuAction>,
        point: Option<[f32; 2]>,
        held: bool,
        pressed: bool,
        modifiers: [bool; 3],
    ) -> bool {
        if self.timeline_pointer(hovered, point, held, pressed, modifiers) {
            return true;
        }
        if !held {
            self.scrub = None;
            return false;
        }
        if pressed
            && let Some(launcher::menu::MenuAction::Studio(Action::Scrub { left, width })) = hovered
        {
            self.scrub = Some((left, width));
            self.transport.pause();
        }
        let Some((left, width)) = self.scrub else {
            return false;
        };
        if let Some([x, _]) = point {
            let fraction =
                ((f64::from(x) - f64::from(left)) / f64::from(width.max(1))).clamp(0.0, 1.0);
            let at = self
                .view
                .visible_start
                .0
                .saturating_add((fraction * self.view.visible_duration.0 as f64).round() as u64);
            let target = MediaTime(at.min(recording_studio::time::MAX_MEDIA_MICROS));
            if let Ok(project) = self.project() {
                let snapped = project
                    .export
                    .fps
                    .snap(target)
                    .unwrap_or(target)
                    .min(project.duration());
                if snapped != self.transport.position() {
                    self.activate(Action::Seek(snapped));
                }
            }
        }
        true
    }

    /// Modal input and desktop focus loss release the editor's pointer capture.
    pub fn cancel_pointer(&mut self) {
        self.release_camera();
        self.scrub = None;
        self.gesture = None;
        Arc::make_mut(&mut self.view).workspace.drag = None;
        self.clear_timeline_drag();
    }
}
