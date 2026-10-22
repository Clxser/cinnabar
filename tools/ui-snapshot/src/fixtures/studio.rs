//! Anonymous editable project metadata exercises Studio without media IO or a live account.

mod states;

pub(super) use states::STATES;

use super::*;
use recording_studio::{
    Id, MediaTime, Project, Span,
    camera::{CameraKey, CameraMode, CameraPose, CameraShot, SavedCameraView},
    effect::{Curve, EffectClip, EffectKind},
    model::{AudioClip, AudioRole, Clip, MediaKind, MediaSource},
};

/// A populated timeline and selected clip expose the production inspector and compact workspace.
pub(super) fn editor() -> Fixture {
    Fixture::Menu(Box::new(view(false)))
}

/// Comfortable controls and the largest timeline stress transport reachability in short windows.
pub(super) fn transport_comfortable() -> Fixture {
    let mut view = view(false);
    let workspace = &mut Arc::make_mut(&mut view.studio).workspace;
    workspace.density = launcher::menu::studio::Density::Comfortable;
    workspace.timeline_size = launcher::menu::studio::TimelineSize::Large;
    Fixture::Menu(Box::new(view))
}

/// A selected world camera exposes its real numeric controls without loading a recording.
pub(super) fn camera() -> Fixture {
    let mut view = camera_view();
    let studio = Arc::make_mut(&mut view.studio);
    studio.camera_workspace.tools = launcher::menu::studio::CameraTools::Views;
    studio.workspace.panel = launcher::menu::studio::EditorPanel::Media;
    Fixture::Menu(Box::new(view))
}

/// Two adjacent anonymous rigs expose saved entry controls without claiming decoded replay evidence.
pub(super) fn camera_blend() -> Fixture {
    use recording_studio::camera::{CameraBlend, CameraRigKind};
    let mut view = camera_view();
    let studio = Arc::make_mut(&mut view.studio);
    let project = Arc::make_mut(studio.project.as_mut().unwrap());
    let cut = MediaTime(7_500_000);
    project.shots[0].span.start = cut;
    project.shots[0].blend_in = Some(CameraBlend {
        duration: MediaTime(1_000_000),
        easing: recording_studio::effect::Easing::Smooth,
    });
    project.shots.push(
        CameraShot::from_preview(
            Id(7),
            Span::new(MediaTime::ZERO, cut).unwrap(),
            CameraRigKind::Free,
            CameraPose {
                position: [-4.0, 68.0, -10.0],
                yaw: 35.0,
                ..Default::default()
            },
            None,
        )
        .unwrap(),
    );
    project.validate().unwrap();
    studio.lanes = Arc::new(recording_studio::lanes::TimelineLanes::new(project).unwrap());
    studio.workspace.panel = launcher::menu::studio::EditorPanel::Inspector;
    studio.workspace.media_hidden = true;
    Fixture::Menu(Box::new(view))
}

/// Anonymous matching players expose the real search field, bounded result page and paging controls.
pub(super) fn camera_subjects() -> Fixture {
    use launcher::menu::studio::{CameraSubjectResults, EditorPanel};
    use recording_studio::camera::{
        ActorPose, CameraSubjectSearch, CameraTarget, CameraTargetKind,
    };
    let mut view = camera_view();
    let studio = Arc::make_mut(&mut view.studio);
    studio.workspace.panel = EditorPanel::Media;
    let preview = studio.preview.camera.as_mut().unwrap();
    preview.targets = ["Amber", "Birch", "Cedar", "Flint", "Hazel", "Willow"]
        .into_iter()
        .enumerate()
        .map(|(index, name)| CameraTarget {
            runtime_id: index as u64 + 2,
            unique_id: index as i64 + 2,
            spawn_revision: 1,
            name: Arc::from(format!("Scout {name}")),
            kind: CameraTargetKind::Player,
            entity_type: Some(Arc::from("minecraft:player")),
            pose: ActorPose {
                position: [index as f64, 64.0, 8.0],
                velocity: [0.0; 3],
                yaw: 0.0,
                pitch: 0.0,
                roll: 0.0,
                eye_offset: Some([0.0, 1.6, 0.0]),
            },
            teleported: false,
        })
        .collect();
    studio.camera_target = Some(preview.targets[0].reference());
    let workspace = &mut studio.camera_workspace;
    workspace.subject_search = CameraSubjectSearch::new("Scout").unwrap();
    workspace.subject_results = Some(CameraSubjectResults::new(
        preview,
        &workspace.subject_search,
        workspace.subject_filter,
    ));
    Fixture::Menu(Box::new(view))
}

/// A selected distance rail exposes travel, orientation and lens controls in the real inspector.
pub(super) fn path() -> Fixture {
    let mut view = camera_view();
    let studio = Arc::make_mut(&mut view.studio);
    let project = Arc::make_mut(studio.project.as_mut().unwrap());
    let shot = &mut project.shots[0];
    shot.mode = CameraMode::Path {
        keys: [
            (MediaTime::ZERO, [2.0, 67.0, -6.0]),
            (MediaTime(15_000_000), [8.0, 70.0, -2.0]),
            (MediaTime(30_000_000), [12.0, 67.0, 3.0]),
        ]
        .map(|(at, position)| CameraKey {
            at,
            pose: CameraPose {
                position,
                ..Default::default()
            },
            easing: recording_studio::effect::Easing::Linear,
            handles: None,
        })
        .into(),
        smooth_position: true,
    };
    shot.path_timing = recording_studio::camera::PathTiming::GlobalDistance;
    studio.lanes = Arc::new(recording_studio::lanes::TimelineLanes::new(project).unwrap());
    Fixture::Menu(Box::new(view))
}

/// An exact authored key exposes rotation rings over original generated preview artwork.
pub(super) fn rotation(path_camera: bool) -> Fixture {
    let Fixture::Menu(mut view) = (if path_camera {
        path()
    } else {
        Fixture::Menu(Box::new(camera_view()))
    }) else {
        unreachable!()
    };
    let studio = Arc::make_mut(&mut view.studio);
    studio.workspace.viewport_tool = launcher::menu::studio::ViewportTool::Rotate;
    studio.workspace.media_hidden = true;
    studio.workspace.inspector_hidden = true;
    studio.camera_workspace.focused = true;
    if path_camera {
        studio.position = MediaTime(15_000_000);
        studio.camera_workspace.key = Some((Id(4), studio.position));
        let pose = studio.project.as_ref().unwrap().shots[0]
            .rotation_pose(studio.position)
            .unwrap();
        let preview = studio.preview.camera.as_mut().unwrap();
        preview.source_time = studio.position;
        preview.pose = pose;
        preview.render_pose = pose;
    }
    Fixture::Menu(view)
}

/// The Assist toolbox is opened explicitly instead of capturing the default hidden panel.
pub(super) fn assist() -> Fixture {
    let mut view = view(false);
    let studio = Arc::make_mut(&mut view.studio);
    studio.workspace.media_hidden = false;
    studio.workspace.toolbox = launcher::menu::studio::Toolbox::Creative;
    studio.workspace.panel = launcher::menu::studio::EditorPanel::Media;
    Fixture::Menu(Box::new(view))
}

/// Camera suggestions remain accessible without scrolling through rhythm controls.
pub(super) fn assist_cameras() -> Fixture {
    let mut view = camera_view();
    let studio = Arc::make_mut(&mut view.studio);
    studio.workspace.toolbox = launcher::menu::studio::Toolbox::Creative;
    studio.workspace.panel = launcher::menu::studio::EditorPanel::Media;
    studio.workspace.inspector_hidden = true;
    studio.workflow.creative.section = launcher::menu::studio::AssistSection::Cameras;
    Fixture::Menu(Box::new(view))
}

/// Supplies only anonymous camera metadata; it does not claim decoded replay or runtime evidence.
fn camera_view() -> MenuView {
    let mut view = view(true);
    let studio = Arc::make_mut(&mut view.studio);
    let project = Arc::make_mut(studio.project.as_mut().unwrap());
    project.audio.clear();
    let source = &mut project.sources[0];
    source.kind = MediaKind::Replay;
    source.path = "Showcase.cinreplay".into();
    let pose = CameraPose {
        position: [2.0, 67.0, -6.0],
        ..Default::default()
    };
    project
        .camera_views
        .push(SavedCameraView::new(Id(6), "Courtyard wide".into(), source, pose).unwrap());
    project.validate().unwrap();
    studio.camera_workspace.saved_views.selected = Some(Id(6));
    studio.workspace.media_hidden = false;
    studio.workspace.inspector_hidden = false;
    studio.workspace.toolbox = launcher::menu::studio::Toolbox::Cameras;
    studio.workspace.panel = launcher::menu::studio::EditorPanel::Inspector;
    studio.preview.ready = true;
    studio.preview.camera = Some(recording_studio::preview::CameraPreview {
        world_labels: Default::default(),
        epoch: [0; 3],
        source: Id(1),
        source_time: studio.position,
        pose,
        render_pose: pose,
        interactive: true,
        viewport: [0.0, 0.0, 1.0, 1.0],
        local_actor: 1,
        targets: Arc::default(),
    });
    view
}

/// Active capture and export notices retain enough editor space for navigation.
pub(super) fn recording() -> Fixture {
    let mut view = view(false);
    let studio = Arc::make_mut(&mut view.studio);
    studio.capture.phase = recording_studio::capture::CapturePhase::Recording;
    studio.capture.elapsed = MediaTime(83_000_000);
    studio.capture.captured_frames = 4_980;
    Fixture::Menu(Box::new(view))
}

/// Audio properties remain directly accessible beside the same timeline.
pub(super) fn audio() -> Fixture {
    let mut view = view(false);
    let studio = Arc::make_mut(&mut view.studio);
    studio.selected = Some(Id(3));
    studio.workspace.inspector_hidden = false;
    studio.workspace.panel = launcher::menu::studio::EditorPanel::Inspector;
    Fixture::Menu(Box::new(view))
}

/// Capture and output settings use full-height groups at both desktop and narrow sizes.
pub(super) fn settings() -> Fixture {
    let mut view = view(false);
    Arc::make_mut(&mut view.studio).workflow.show_settings = true;
    Fixture::Menu(Box::new(view))
}

/// Quality settings remain readable without allocating a renderer or opening any source.
pub(super) fn preview_settings() -> Fixture {
    let mut view = view(false);
    let studio = Arc::make_mut(&mut view.studio);
    studio.workflow.show_settings = true;
    studio.workspace.settings_page = launcher::menu::studio::SettingsPage::Preview;
    Fixture::Menu(Box::new(view))
}

/// Help reads the same command registry as the production keyboard handler.
pub(super) fn controls() -> Fixture {
    let mut view = view(false);
    let studio = Arc::make_mut(&mut view.studio);
    studio.workflow.show_settings = true;
    studio.workspace.settings_page = launcher::menu::studio::SettingsPage::Controls;
    Fixture::Menu(Box::new(view))
}

/// Dependency failure guidance uses no real install path, device name or subprocess.
pub(super) fn setup() -> Fixture {
    let mut view = view(false);
    let studio = Arc::make_mut(&mut view.studio);
    studio.workflow.show_settings = true;
    studio.workspace.settings_page = launcher::menu::studio::SettingsPage::Diagnostics;
    studio.workflow.diagnostics.error = Some("FFmpeg was not found. Install FFmpeg with FFprobe, restart Cinnabar, then check dependencies again.".into());
    Fixture::Menu(Box::new(view))
}

/// Recovery controls expose an admitted copy and a damaged entry without touching storage.
pub(super) fn recovery() -> Fixture {
    let mut view = view(false);
    let recovery = &mut Arc::make_mut(&mut view.studio).workflow.recovery;
    recovery.visible = true;
    recovery.entries = Arc::from([
        recording_studio::storage::RecoveryEntry {
            id: "fixture-session".into(),
            name: "Courtyard edit".into(),
            original_file: None,
            revision: 8,
            discard_through: 8,
            modified_unix_ms: 0,
            versions: 3,
            notice: None,
            recoverable: true,
        },
        recording_studio::storage::RecoveryEntry {
            id: "fixture-damaged".into(),
            name: "Interrupted project".into(),
            original_file: None,
            revision: 2,
            discard_through: 2,
            modified_unix_ms: 0,
            versions: 1,
            notice: Some("Recovery copy is damaged. Your saved project is unchanged.".into()),
            recoverable: false,
        },
    ]);
    Fixture::Menu(Box::new(view))
}

/// Produces admitted original fixture data with no captured identifiers, paths or artwork.
fn view(camera: bool) -> MenuView {
    let mut project = Project::new("Showcase edit");
    let span = Span::new(MediaTime::ZERO, MediaTime(30_000_000)).unwrap();
    project.sources.push(MediaSource {
        id: Id(1),
        name: "Showcase recording".into(),
        path: "Showcase.mp4".into(),
        kind: MediaKind::Video,
        duration: span.duration(),
        size: [1920, 1080],
        frame_rate: Some(Default::default()),
        sha256: None,
    });
    project.clips.push(Clip {
        id: Id(2),
        source: Id(1),
        source_range: span,
        at: MediaTime::ZERO,
        rate: Default::default(),
        speed_ramp: None,
        track: 0,
        framing: Default::default(),
        enabled: true,
    });
    project.audio.push(AudioClip {
        id: Id(3),
        source: Id(1),
        linked_clip: Some(Id(2)),
        source_range: span,
        at: MediaTime::ZERO,
        rate: Default::default(),
        speed_ramp: None,
        role: AudioRole::Game,
        gain: 1.0,
        fade_in: MediaTime::ZERO,
        fade_out: MediaTime::ZERO,
        muted: false,
    });
    project.shots.push(CameraShot {
        id: Id(4),
        name: "Courtyard camera".into(),
        span,
        mode: CameraMode::Free {
            pose: CameraPose {
                position: [2.0, 67.0, -6.0],
                ..Default::default()
            },
        },
        hide_hud: true,
        hide_hand: true,
        blend_in: None,
        target_lifetime: None,
        fov_curve: None,
        animation: Default::default(),
        fov_offset: MediaTime::ZERO,
        path_timing: Default::default(),
        path_offset: MediaTime::ZERO,
        path_global_easing: recording_studio::effect::Easing::Linear,
        path_continuity: Default::default(),
    });
    project.effects.push(EffectClip {
        id: Id(5),
        span,
        kind: EffectKind::Contrast,
        amount: Curve::constant(0.15),
        enabled: true,
        origin: None,
    });
    project
        .validate()
        .expect("Studio fixture must be an admitted edit");
    let mut view = MenuView::new(true, "Player".into());
    view.screen = MenuScreen::RecordingStudio;
    let studio = Arc::make_mut(&mut view.studio);
    studio.show_library = false;
    studio.selected = Some(Id(if camera { 4 } else { 2 }));
    studio.position = MediaTime(8_000_000);
    studio.visible_duration = span.duration();
    studio.lanes = Arc::new(recording_studio::lanes::TimelineLanes::new(&project).unwrap());
    studio.project = Some(Arc::new(project));
    studio.undo = Some("Add camera".into());
    view
}
