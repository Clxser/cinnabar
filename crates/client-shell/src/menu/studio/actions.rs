//! Native intent dispatch keeps editing owners separate from capture and transport state.

use super::*;

impl Studio {
    /// Applies one action and returns a user-visible error instead of swallowing stale selections.
    pub(super) fn apply_action(&mut self, action: Action) -> Result<(), String> {
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
            Action::Graph(action) => self.graph_action(action)?,
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
}
