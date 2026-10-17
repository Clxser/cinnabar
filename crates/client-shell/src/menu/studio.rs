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

mod bridge;
mod camera;
mod clips;
mod effects;
mod media;
mod scene;
mod settings;
mod speed;
pub(super) mod timeline;
mod titles;

pub(super) struct Studio {
    pub view: Arc<Snapshot>,
    root: PathBuf,
    storage: Option<ProjectStorage>,
    editor: Option<Editor>,
    transport: Transport,
    library_job: Option<u64>,
    open_job: Option<u64>,
    save_job: Option<(u64, u64)>,
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
    preferences_job: Option<u64>,
    preferences_changed: bool,
    thumbnails_job: Option<u64>,
    thumbnails_cancel: recording_studio::export::Cancellation,
    open_cancel: recording_studio::export::Cancellation,
    pub(super) destination_text: ui::ChatEditor,
    pub(super) camera_text: ui::ChatEditor,
    camera_edit: Option<camera::CameraValueEdit>,
    pub(super) clip_text: ui::ChatEditor,
    clip_edit: Option<clips::ClipTimeEdit>,
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
            library_job: None,
            open_job: None,
            save_job: None,
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
            preferences_job: None,
            preferences_changed: false,
            thumbnails_job: None,
            thumbnails_cancel: Default::default(),
            open_cancel: Default::default(),
            destination_text: super::input::fields::field_editor(
                launcher::menu::MenuField::StudioDestination,
            ),
            camera_text: super::input::fields::field_editor(
                launcher::menu::MenuField::StudioCamera(
                    recording_studio::camera::CameraParameter::X,
                ),
            ),
            camera_edit: None,
            clip_text: super::input::fields::field_editor(launcher::menu::MenuField::StudioClip(
                launcher::menu::studio::ClipTimeField::Position,
            )),
            clip_edit: None,
        }
    }

    /// Opens the existing document or library without resetting unsaved edits.
    pub fn open(&mut self) {
        self.scrub = None;
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
            self.transport
                .advance_until(elapsed, editor.duration(), available);
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
        let mut actions = if self.view.discard.is_some() {
            vec![Action::CancelDiscard, Action::ConfirmDiscard]
        } else if self.view.loading {
            let mut actions = vec![Action::CancelOpen];
            if self.view.capture.phase.busy() {
                actions.push(Action::ToggleRecording);
            }
            actions
        } else if self.view.show_library || self.editor.is_none() {
            let mut actions = vec![Action::NewProject, Action::Refresh];
            actions.extend((0..self.view.library.len()).map(Action::Open));
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
            && self.editor.is_some()
        {
            actions.extend(self.camera_focus_actions());
            actions.extend(self.clip_focus_actions());
            actions.extend(self.speed_focus_actions());
            actions.extend(self.effect_focus_actions());
            actions.extend(self.title_focus_actions());
        }
        if self.view.discard.is_none() && !self.view.loading {
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
            Arc::make_mut(&mut self.view).speed_key = None;
        }
        match action {
            Action::ImportMedia => self.begin_media(false)?,
            Action::RelinkMedia => self.begin_media(true)?,
            Action::CancelMedia => self.cancel_media(),
            Action::ToggleSettings => {
                let workflow = &mut Arc::make_mut(&mut self.view).workflow;
                workflow.show_settings = !workflow.show_settings;
            }
            Action::CycleCaptureFps => self.cycle_capture_fps()?,
            Action::CycleCaptureSize => self.cycle_capture_size()?,
            Action::ToggleCaptureHand => self.toggle_capture_hand()?,
            Action::ToggleCaptureHud => self.toggle_capture_hud()?,
            Action::CycleExportFps
            | Action::CycleExportSize
            | Action::CycleExportCodec
            | Action::CycleExportEncoder
            | Action::AdjustExportQuality(_)
            | Action::ToggleExportAudio => self.output_setting(action)?,
            Action::EditExportDestination => self.begin_destination(),
            Action::ApplyExportDestination => self.apply_destination()?,
            Action::CancelExportDestination => {
                Arc::make_mut(&mut self.view).workflow.destination_draft = None
            }
            Action::Export => self.queue_export()?,
            Action::CancelExport => self.export_cancel = true,
            Action::ToggleRecording => self.queue_capture_toggle(),
            Action::Bookmark => {
                if self.capture_request.is_none()
                    && self.view.capture.phase == recording_studio::capture::CapturePhase::Recording
                {
                    self.capture_request =
                        Some(recording_studio::capture::CaptureRequest::Bookmark);
                }
            }
            Action::NewProject => self.leave(DiscardIntent::NewProject, false)?,
            Action::Open(index) => self.leave(DiscardIntent::Open(index), false)?,
            Action::ConfirmDiscard => {
                if let Some(intent) = Arc::make_mut(&mut self.view).discard.take() {
                    self.leave(intent, true)?;
                }
            }
            Action::CancelDiscard => Arc::make_mut(&mut self.view).discard = None,
            Action::CancelOpen => {
                self.open_cancel.cancel();
                self.open_job = None;
                Arc::make_mut(&mut self.view).loading = false;
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
            Action::DeleteSelected => {
                let id = self
                    .view
                    .selected
                    .ok_or("Select a timeline item to delete.")?;
                self.edit("Delete item", &[Command::Delete { id }])?;
                Arc::make_mut(&mut self.view).selected = None;
            }
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
            Action::AdjustEffect(direction) => self.adjust_effect(direction)?,
            Action::AddEffectKey => self.key_effect()?,
            Action::DeleteEffectKey => self.delete_effect_key()?,
            Action::CycleEffectEasing => self.cycle_effect_easing()?,
            Action::StepEffectKey(direction) => self.step_effect_key(direction)?,
            Action::SelectEffectKey { effect, at } => self.select_effect_key(effect, at)?,
            Action::MoveEffect(frames) => self.move_effect(frames)?,
            Action::UseNativeReplay => self.use_native_replay()?,
            Action::CreateCamera(kind) => self.create_camera(kind)?,
            Action::CycleCameraTarget(direction) => self.cycle_camera_target(direction),
            Action::RetargetCamera => self.retarget_camera()?,
            Action::AdjustCamera {
                parameter,
                direction,
            } => self.adjust_camera(parameter, direction)?,
            Action::AddCameraKey => self.add_camera_key()?,
            Action::DeleteCameraKey => self.delete_camera_key()?,
            Action::CycleCameraEasing => self.cycle_camera_easing()?,
            Action::ToggleCameraSmoothing => self.toggle_camera_smoothing()?,
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
        match intent {
            DiscardIntent::NewProject => {
                let editor =
                    Editor::new(Project::new("Untitled project")).map_err(|e| e.to_string())?;
                self.install(editor, None);
            }
            DiscardIntent::Open(index) => {
                self.ensure_storage()?;
                let entry = self
                    .view
                    .library
                    .get(index)
                    .ok_or("This library item is no longer available. Refresh the library.")?;
                if entry.interrupted && self.view.capture.phase.busy() {
                    return Err(
                        "Stop and finish recording before attempting interrupted-take recovery."
                            .into(),
                    );
                }
                self.thumbnails_cancel.cancel();
                self.open_cancel.cancel();
                self.open_cancel = Default::default();
                let request = match entry.kind {
                    LibraryKind::Project => StorageRequest::LoadProject {
                        path: entry.path.clone(),
                    },
                    LibraryKind::Recording if entry.interrupted => {
                        StorageRequest::RecoverRecording {
                            path: entry.path.clone(),
                            cancellation: self.open_cancel.clone(),
                        }
                    }
                    LibraryKind::Recording => StorageRequest::OpenRecording {
                        path: entry.path.clone(),
                    },
                };
                self.open_job = Some(
                    self.storage
                        .as_mut()
                        .expect("storage exists")
                        .request(request)
                        .map_err(|e| e.to_string())?,
                );
                Arc::make_mut(&mut self.view).loading = true;
            }
        }
        Ok(())
    }

    /// Starts the service lazily so ordinary gameplay and launcher startup incur no studio IO.
    fn ensure_storage(&mut self) -> Result<(), String> {
        if self.storage.is_none() {
            self.storage = Some(ProjectStorage::start().map_err(|e| e.to_string())?);
        }
        Ok(())
    }

    /// Coalesces repeated refresh presses into one outstanding library job.
    fn refresh(&mut self) -> Result<(), String> {
        if self.library_job.is_some() {
            return Ok(());
        }
        self.ensure_storage()?;
        self.thumbnails_cancel.cancel();
        self.thumbnails_job = None;
        self.library_job = Some(
            self.storage
                .as_mut()
                .expect("storage exists")
                .request(StorageRequest::ListLibrary {
                    root: self.root.clone(),
                })
                .map_err(|e| e.to_string())?,
        );
        Arc::make_mut(&mut self.view).library_loading = true;
        Ok(())
    }

    /// Captures an immutable revision; opening another document cannot misapply its save completion.
    fn save(&mut self) -> Result<(), String> {
        if self.save_job.is_some() {
            return Ok(());
        }
        self.ensure_storage()?;
        self.thumbnails_cancel.cancel();
        let editor = self
            .editor
            .as_ref()
            .ok_or("Create or open a project first.")?;
        let project = Arc::clone(editor.project());
        let revision = editor.revision();
        let request = if let Some(path) = &self.view.file {
            StorageRequest::SaveProject {
                path: path.clone(),
                project,
                revision,
                source_base: self.root.clone(),
            }
        } else {
            StorageRequest::SaveNewProject {
                directory: self.root.clone(),
                project,
                revision,
                source_base: self.root.clone(),
            }
        };
        let job = self
            .storage
            .as_mut()
            .expect("storage exists")
            .request(request)
            .map_err(|e| e.to_string())?;
        self.save_job = Some((job, self.document));
        Arc::make_mut(&mut self.view).saving = true;
        Ok(())
    }

    /// Publishes only the newest matching job; a failed open leaves the existing editor intact.
    fn completed(&mut self, completed: StorageResult) {
        let library = self.library_job == Some(completed.job);
        let open = self.open_job == Some(completed.job);
        let saved = self.save_job.filter(|(job, _)| *job == completed.job);
        let preferences = self.preferences_job == Some(completed.job);
        let thumbnails = self.thumbnails_job == Some(completed.job);
        if !library && !open && saved.is_none() && !preferences && !thumbnails {
            return;
        }
        if preferences {
            self.preferences_job = None;
        }
        if thumbnails {
            self.thumbnails_job = None;
        }
        if library {
            self.library_job = None;
            Arc::make_mut(&mut self.view).library_loading = false;
        }
        if open {
            self.open_job = None;
            Arc::make_mut(&mut self.view).loading = false;
        }
        if saved.is_some() {
            self.save_job = None;
            Arc::make_mut(&mut self.view).saving = false;
        }
        let value = match completed.result {
            Ok(value) => value,
            Err(error) => {
                bevy::log::warn!(error = %error, "Studio storage operation failed");
                Arc::make_mut(&mut self.view).error = Some(error.to_string());
                return;
            }
        };
        match value {
            StorageValue::Library(listing) => {
                let view = Arc::make_mut(&mut self.view);
                view.library = listing.entries.into();
                view.library_truncated = listing.truncated;
                view.notice = (!listing.issues.is_empty()).then(|| listing.issues.join("\n"));
                view.workflow.thumbnail_sheet = None;
                view.workflow.thumbnail_slots = Arc::default();
                self.thumbnails_cancel = Default::default();
                self.thumbnails_job = self.storage.as_mut().and_then(|storage| {
                    storage
                        .request(StorageRequest::Thumbnails {
                            entries: self.view.library.clone(),
                            cancellation: self.thumbnails_cancel.clone(),
                        })
                        .ok()
                });
            }
            StorageValue::Thumbnails { sheet, slots } => {
                let workflow = &mut Arc::make_mut(&mut self.view).workflow;
                workflow.thumbnail_sheet = Some(sheet);
                workflow.thumbnail_slots = slots.into();
            }
            StorageValue::CaptureSettings(settings) => {
                if !self.preferences_changed {
                    Arc::make_mut(&mut self.view).workflow.capture_settings = settings;
                }
            }
            StorageValue::LoadedProject { path, editor } => self.install(editor, Some(path)),
            StorageValue::CapturedVideo { editor, notice } => {
                self.install(editor, None);
                Arc::make_mut(&mut self.view).notice = notice;
            }
            StorageValue::SavedProject { path, revision } => {
                if saved.is_some_and(|(_, document)| document == self.document) {
                    if let Some(editor) = &mut self.editor {
                        editor.mark_saved(revision);
                    }
                    Arc::make_mut(&mut self.view).file = Some(path);
                    let _ = self.refresh();
                }
            }
            StorageValue::Recording { path, reader } => {
                if let Err(error) = self.import_replay(path, reader) {
                    Arc::make_mut(&mut self.view).error = Some(error);
                }
            }
        }
        self.publish_editor();
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
        if let Some(job) = &self.media_job {
            job.cancellation.cancel();
        }
        self.cancel_title_text();
        self.cancel_camera_value();
        self.cancel_clip_time();
        self.document = self.document.wrapping_add(1);
        self.editor = Some(editor);
        self.replays.clear();
        self.transport = Transport::default();
        let view = Arc::make_mut(&mut self.view);
        view.file = path;
        view.show_library = false;
        view.selected = None;
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
        self.reconcile_clip_time();
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
        editor.apply(label, commands).map_err(|e| e.to_string())?;
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
        x: Option<f32>,
        held: bool,
        pressed: bool,
    ) -> bool {
        if self.timeline_pointer(hovered, x, held, pressed) {
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
        if let Some(x) = x {
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
        self.scrub = None;
        self.gesture = None;
        Arc::make_mut(&mut self.view).workspace.drag = None;
    }
}
