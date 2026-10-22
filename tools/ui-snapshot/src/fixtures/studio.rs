//! Anonymous editable project metadata exercises Studio without media IO or a live account.

use super::*;
use recording_studio::{
    Id, MediaTime, Project, Span,
    camera::{CameraMode, CameraPose, CameraShot},
    effect::{Curve, EffectClip, EffectKind},
    model::{AudioClip, AudioRole, Clip, MediaKind, MediaSource},
};

/// A populated timeline and selected clip expose the production inspector and compact workspace.
pub(super) fn editor() -> Fixture {
    Fixture::Menu(Box::new(view(false)))
}

/// A selected world camera exposes its real numeric controls without loading a recording.
pub(super) fn camera() -> Fixture {
    Fixture::Menu(Box::new(view(true)))
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
    Arc::make_mut(&mut view.studio).selected = Some(Id(3));
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
        target_lifetime: None,
        fov_curve: None,
        fov_offset: MediaTime::ZERO,
        path_timing: Default::default(),
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
