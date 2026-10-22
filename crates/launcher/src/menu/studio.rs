//! Native creator workspace actions and immutable presentation state.

use std::{path::PathBuf, sync::Arc};

use recording_studio::camera::{CameraParameter, CameraRigKind, CameraTargetRef};
use recording_studio::{Id, MediaTime, PlaybackRate, Project, storage::LibraryEntry};

pub const TITLE: &str = "Recording Studio";
pub const TIMELINE_SCROLL_ID: &str = "studio_timeline";

mod clip;
mod export_range;
pub use export_range::ExportRangeField;
pub mod timeline;
pub use clip::{ClipTimeField, MAX_CLIP_TIME_BYTES};
mod camera;
pub mod graph;
pub use camera::{
    CAMERA_SUBJECT_ROWS, CameraBlendAction, CameraGuides, CameraNavigationAction, CameraPathCache,
    CameraRotationHandle, CameraSubjectResults, CameraSubjectsAction, CameraViewNameDraft,
    CameraViewsAction, CameraWorkspace, NavigationGesture, SAVED_CAMERA_VIEWS_PER_PAGE,
    SavedCameraViews,
};
pub mod creative;
mod workspace;
pub use workspace::{
    AssistSection, CameraTools, Density, InspectorSection, SettingsPage, TimelineSize, Toolbox,
    ViewportTool, WorkspaceAction, WorkspaceLayout,
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum EditorPanel {
    #[default]
    Preview,
    Media,
    Inspector,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Workspace {
    pub library: library::LibraryWorkspace,
    pub monitoring: monitoring::MonitoringWorkspace,
    pub timeline: timeline::TimelineWorkspace,
    pub panel: EditorPanel,
    pub toolbox: Toolbox,
    pub inspector: InspectorSection,
    pub density: Density,
    pub timeline_size: TimelineSize,
    pub layout: WorkspaceLayout,
    pub media_hidden: bool,
    pub inspector_hidden: bool,
    pub property_selection: Option<Id>,
    pub preview_only: bool,
    pub viewport_tool: ViewportTool,
    pub preview_fill: bool,
    pub preview_quality: recording_studio::preview::PreviewQuality,
    pub settings_page: SettingsPage,
    pub ripple: bool,
    pub follow_playhead: bool,
    pub drag: Option<(Id, recording_studio::Span)>,
}

impl Default for Workspace {
    /// Opens on a spacious viewer and timeline; tools and properties appear when explicitly requested.
    fn default() -> Self {
        Self {
            library: Default::default(),
            monitoring: Default::default(),
            timeline: Default::default(),
            panel: Default::default(),
            toolbox: Default::default(),
            inspector: Default::default(),
            density: Default::default(),
            timeline_size: Default::default(),
            layout: Default::default(),
            media_hidden: true,
            inspector_hidden: true,
            property_selection: None,
            preview_only: false,
            viewport_tool: Default::default(),
            preview_fill: false,
            preview_quality: Default::default(),
            settings_page: Default::default(),
            ripple: false,
            follow_playhead: false,
            drag: None,
        }
    }
}
mod workflow;
pub use workflow::Workflow;
pub mod library;
pub mod monitoring;
pub mod shortcuts;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    Graph(graph::GraphAction),
    CameraSubjects(CameraSubjectsAction),
    Creative(creative::CreativeAction),
    Monitoring(monitoring::MonitoringAction),
    Timeline(timeline::TimelineAction),
    Workspace(WorkspaceAction),
    ToggleRecording,
    ImportMedia,
    RelinkMedia,
    CancelMedia,
    ToggleSettings,
    ScanRecovery,
    ShowRecovery,
    HideRecovery,
    RestoreRecovery(usize),
    DiscardRecovery(usize),
    RetryAutosave,
    CheckDependencies,
    UseCpuEncoder,
    CycleCaptureMode,
    CycleCaptureFps,
    CycleCaptureSize,
    ToggleCaptureHand,
    ToggleCaptureHud,
    CycleExportFps,
    CycleExportSize,
    CycleExportCodec,
    CycleExportEncoder,
    AdjustExportQuality(i8),
    ToggleExportAudio,
    ToggleRecordedHud,
    EditExportDestination,
    ApplyExportDestination,
    CancelExportDestination,
    FullExportRange,
    SetExportRangeAtPlayhead(ExportRangeField),
    EditExportRange(ExportRangeField),
    ApplyExportRange,
    CancelExportRange,
    Bookmark,
    NewProject,
    Refresh,
    LibraryTools(library::LibraryAction),
    Open {
        generation: u64,
        index: usize,
    },
    Library,
    Save,
    Export,
    CancelExport,
    Undo,
    Redo,
    TogglePlayback,
    Step(i8),
    Seek(MediaTime),
    /// Logical-pixel ruler geometry; pointer input resolves this into a frame-snapped seek.
    Scrub {
        left: i32,
        width: u32,
    },
    Select(Id),
    Panel(EditorPanel),
    ToggleRipple,
    ToggleFollowPlayhead,
    FitTimeline,
    RevealPlayhead,
    TimelineOverview {
        left: i32,
        width: u32,
    },
    DragItem {
        id: Id,
        mode: recording_studio::timeline::DragMode,
        left: i32,
        width: u32,
    },
    Split,
    DeleteSelected,
    SetRate(PlaybackRate),
    EditClipTime(ClipTimeField),
    ApplyClipTime,
    CancelClipTime,
    AdjustFraming {
        parameter: recording_studio::timeline::FramingParameter,
        direction: i8,
    },
    ResetFraming,
    MoveClipLayer(i8),
    TransitionToNext(recording_studio::model::TransitionKind),
    RemoveClipTransition,
    SetSpeedKey {
        rate: PlaybackRate,
        ripple: bool,
    },
    DeleteSpeedKey {
        ripple: bool,
    },
    CycleSpeedEasing {
        ripple: bool,
    },
    ClearSpeedRamp {
        ripple: bool,
    },
    SelectSpeedKey {
        clip: Id,
        at: MediaTime,
    },
    StepSpeedKey(i8),
    AddTitle,
    EditTitleText,
    ApplyTitleText,
    CancelTitleText,
    CycleTitleStyle,
    ToggleTitleShadow,
    AdjustTitle {
        parameter: recording_studio::title::TitleParameter,
        direction: i8,
    },
    TrimTitleStart,
    TrimTitleEnd,
    MoveTitle(i8),
    AddEffect(recording_studio::effect::EffectKind),
    AddLook(recording_studio::effect::preset::Look),
    ToggleEffect,
    AdjustEffect(i8),
    AddEffectKey,
    DeleteEffectKey,
    CycleEffectEasing,
    StepEffectKey(i8),
    SelectEffectKey {
        effect: Id,
        at: MediaTime,
    },
    MoveEffect(i8),
    ToggleAudioMute,
    AdjustAudio {
        parameter: recording_studio::audio::AudioParameter,
        direction: i8,
    },
    UseNativeReplay,
    CameraViewport {
        bounds: [i32; 4],
    },
    CameraControl {
        shot: Id,
        at: MediaTime,
        handle: recording_studio::camera::PathControl,
        bounds: [i32; 4],
    },
    CameraSubject(CameraTargetRef),
    CameraRotationControl {
        shot: Id,
        at: MediaTime,
        axis: recording_studio::camera::CameraRotationAxis,
        bounds: [i32; 4],
    },
    CameraAxisControl {
        shot: Id,
        at: MediaTime,
        handle: recording_studio::camera::PathControl,
        axis: recording_studio::camera::CameraDragAxis,
        bounds: [i32; 4],
    },
    SetCameraDragAxis(recording_studio::camera::CameraDragAxis),
    CycleCameraSnap,
    CycleCameraGuides,
    FrameCameraSubject,
    CameraViews(CameraViewsAction),
    CameraNavigation(CameraNavigationAction),
    CameraBlend(CameraBlendAction),
    ReleaseCamera,
    ApplyCameraView,
    ResetCameraView,
    ResetCameraHandles,
    CreateCamera(CameraRigKind),
    CycleCameraTarget(i8),
    ToggleCameraSubjectFilter,
    RetargetCamera,
    AdjustCamera {
        parameter: CameraParameter,
        direction: i8,
    },
    AddCameraKey,
    DeleteCameraKey,
    AddCameraFovKey,
    DeleteCameraFovKey,
    ClearCameraFov,
    CycleCameraFovEasing,
    StepCameraFovKey(i8),
    SelectCameraFovKey {
        shot: Id,
        at: MediaTime,
    },
    CycleCameraEasing,
    ToggleCameraSmoothing,
    ToggleCameraDistanceTiming,
    CycleCameraGlobalEasing,
    ToggleCameraContinuity,
    StepCameraKey(i8),
    SelectCameraKey {
        shot: Id,
        at: MediaTime,
    },
    MoveCameraKey(i8),
    EditCameraValue(CameraParameter),
    ApplyCameraValue,
    CancelCameraValue,
    TrimCameraStart,
    TrimCameraEnd,
    ToggleCameraFine,
    Zoom(i8),
    Pan(i8),
    ConfirmDiscard,
    CancelDiscard,
    CancelOpen,
    ClearError,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DiscardIntent {
    NewProject,
    Open {
        path: PathBuf,
        kind: recording_studio::storage::LibraryKind,
        interrupted: bool,
    },
    RestoreRecovery(String),
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub workspace: Workspace,
    pub camera_workspace: CameraWorkspace,
    pub workflow: Workflow,
    pub capture: recording_studio::capture::CaptureStatus,
    pub export: recording_studio::export::ExportStatus,
    pub library: Arc<[LibraryEntry]>,
    pub library_loading: bool,
    pub library_truncated: bool,
    pub show_library: bool,
    pub project: Option<Arc<Project>>,
    pub lanes: Arc<recording_studio::lanes::TimelineLanes>,
    pub file: Option<PathBuf>,
    pub position: MediaTime,
    pub playing: bool,
    pub dirty: bool,
    pub undo: Option<String>,
    pub redo: Option<String>,
    pub selected: Option<Id>,
    pub camera_target: Option<CameraTargetRef>,
    pub camera_fine: bool,
    pub camera_draft: Option<(Id, MediaTime, CameraParameter, Arc<str>)>,
    pub clip_draft: Option<(Id, ClipTimeField, Arc<str>)>,
    pub speed_key: Option<(Id, MediaTime)>,
    pub title_draft: Option<(Id, Arc<str>)>,
    pub visible_start: MediaTime,
    pub visible_duration: MediaTime,
    pub saving: bool,
    pub loading: bool,
    pub discard: Option<DiscardIntent>,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub preview: recording_studio::preview::PreviewStatus,
    pub audio: recording_studio::audio::PreviewStatus,
    pub preview_icon: Option<ui::IconRef>,
}

impl PartialEq for Snapshot {
    /// Project and library payloads are immutable publications; equality never traverses the timeline.
    fn eq(&self, other: &Self) -> bool {
        let Self {
            workspace,
            camera_workspace,
            workflow,
            capture,
            export,
            library,
            library_loading,
            library_truncated,
            show_library,
            project,
            lanes,
            file,
            position,
            playing,
            dirty,
            undo,
            redo,
            selected,
            camera_target,
            camera_fine,
            camera_draft,
            clip_draft,
            speed_key,
            title_draft,
            visible_start,
            visible_duration,
            saving,
            loading,
            discard,
            error,
            notice,
            preview,
            audio,
            preview_icon,
        } = self;
        let Self {
            workspace: b_workspace,
            camera_workspace: b_camera_workspace,
            workflow: b_workflow,
            capture: b_capture,
            export: b_export,
            library: b_library,
            library_loading: b_library_loading,
            library_truncated: b_library_truncated,
            show_library: b_show_library,
            project: b_project,
            lanes: b_lanes,
            file: b_file,
            position: b_position,
            playing: b_playing,
            dirty: b_dirty,
            undo: b_undo,
            redo: b_redo,
            selected: b_selected,
            camera_target: b_camera_target,
            camera_fine: b_camera_fine,
            camera_draft: b_camera_draft,
            clip_draft: b_clip_draft,
            speed_key: b_speed_key,
            title_draft: b_title_draft,
            visible_start: b_visible_start,
            visible_duration: b_visible_duration,
            saving: b_saving,
            loading: b_loading,
            discard: b_discard,
            error: b_error,
            notice: b_notice,
            preview: b_preview,
            audio: b_audio,
            preview_icon: b_preview_icon,
        } = other;
        workspace == b_workspace
            && camera_workspace == b_camera_workspace
            && capture == b_capture
            && workflow == b_workflow
            && (Arc::ptr_eq(lanes, b_lanes) || lanes.is_empty() && b_lanes.is_empty())
            && export == b_export
            && camera_target == b_camera_target
            && camera_fine == b_camera_fine
            && camera_draft == b_camera_draft
            && clip_draft == b_clip_draft
            && speed_key == b_speed_key
            && title_draft == b_title_draft
            && preview == b_preview
            && audio == b_audio
            && preview_icon == b_preview_icon
            && (Arc::ptr_eq(library, b_library) || library.is_empty() && b_library.is_empty())
            && match (project, b_project) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
            && (
                library_loading,
                library_truncated,
                show_library,
                file,
                position,
                playing,
                dirty,
                undo,
                redo,
            ) == (
                b_library_loading,
                b_library_truncated,
                b_show_library,
                b_file,
                b_position,
                b_playing,
                b_dirty,
                b_undo,
                b_redo,
            )
            && (
                selected,
                visible_start,
                visible_duration,
                saving,
                loading,
                discard,
                error,
                notice,
            ) == (
                b_selected,
                b_visible_start,
                b_visible_duration,
                b_saving,
                b_loading,
                b_discard,
                b_error,
                b_notice,
            )
    }
}
