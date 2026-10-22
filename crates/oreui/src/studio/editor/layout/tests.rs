use super::*;
use launcher::menu::studio::{
    Density, EditorPanel, TimelineSize, monitoring::MonitoringAction, timeline::TimelineAction,
};
use recording_studio::{
    Id, MediaTime, PlaybackRate, Project, Span,
    camera::{CameraPose, CameraRigKind, CameraShot},
    effect::{Curve, EffectClip, EffectKind},
    model::{Clip, Framing, MediaKind, MediaSource},
    preview::CameraPreview,
    tracks::Track,
};
use std::{collections::HashMap, sync::Arc};

/// Video, camera and effects contain admitted content; empty authoring rows remain owned by TimelineLanes.
fn fixture(overlaps: usize) -> MenuView {
    let mut project = Project::new("Preview geometry");
    let span = Span::new(MediaTime::ZERO, MediaTime(60_000_000)).unwrap();
    project.sources.push(MediaSource {
        id: Id(1),
        name: "Replay".into(),
        path: "take.cinreplay".into(),
        kind: MediaKind::Replay,
        duration: span.end,
        size: [1600, 900],
        frame_rate: None,
        sha256: None,
    });
    project.clips.push(Clip {
        id: Id(2),
        source: Id(1),
        source_range: span,
        at: MediaTime::ZERO,
        rate: PlaybackRate::default(),
        speed_ramp: None,
        track: 0,
        framing: Framing::default(),
        enabled: true,
    });
    project.shots.push(
        CameraShot::from_preview(
            Id(3),
            span,
            CameraRigKind::Free,
            CameraPose::default(),
            None,
        )
        .unwrap(),
    );
    for index in 0..overlaps.max(1) {
        project.effects.push(EffectClip {
            id: Id(4 + index as u64),
            span,
            kind: EffectKind::Contrast,
            amount: Curve::constant(0.1),
            enabled: true,
            origin: None,
        });
    }
    project.validate().unwrap();
    let mut view = MenuView::new(true, "Fixture".into());
    let studio = Arc::make_mut(&mut view.studio);
    studio.show_library = false;
    studio.workspace.media_hidden = true;
    studio.workspace.inspector_hidden = true;
    studio.workspace.density = Density::Comfortable;
    studio.workspace.timeline_size = TimelineSize::Large;
    studio.position = MediaTime(8_000_000);
    studio.visible_duration = span.duration();
    studio.workspace.monitoring.seek_draft = Some("8".into());
    studio.lanes = Arc::new(recording_studio::lanes::TimelineLanes::new(&project).unwrap());
    studio.project = Some(Arc::new(project));
    studio.preview.ready = true;
    studio.preview_icon = Some(ui::IconRef {
        page: 57,
        uv: [0, 0, 1600, 900],
        glint: false,
    });
    studio.preview.camera = Some(CameraPreview {
        world_labels: Default::default(),
        epoch: [0; 3],
        source: Id(1),
        source_time: studio.position,
        pose: CameraPose::default(),
        render_pose: CameraPose::default(),
        interactive: true,
        viewport: [0.0, 0.0, 1.0, 1.0],
        local_actor: 0,
        targets: Arc::default(),
    });
    view
}

/// Checks actual image hits and painted lane scroll geometry with the window's native text metrics.
fn assert_paint(
    view: &MenuView,
    size: [f32; 2],
    minimum_picture: f32,
) -> ui_engine::scroll::ScrollArea {
    let mut full = Vec::new();
    let (scrolls, hits, _) = crate::review_tests::paint(HashMap::new(), |canvas| {
        let metrics = ui_engine::text_metrics::TextMetrics::for_viewport(
            size.map(|v| v as u32),
            ui::DpiScale::new(1.0).unwrap(),
            Some(2),
        );
        canvas.metrics = metrics;
        canvas.rem = metrics.scale.get()
            * ui::FONT_DESIGN_PIXEL_TEXELS as f32
            * crate::theme::GUI_PIXELS_PER_REM;
        canvas.capture_focus = true;
        crate::studio::draw(canvas, view, size).unwrap();
        full = canvas.focus_hits.clone();
    });
    let picture = hits
        .iter()
        .find_map(|(action, bounds)| {
            matches!(action, MenuAction::Studio(Action::CameraViewport { .. })).then_some(*bounds)
        })
        .expect("native image stays visible");
    assert!(
        picture.height() >= minimum_picture,
        "picture {} at {size:?}, bounds {picture:?}, scrolls {scrolls:?}",
        picture.height()
    );
    let timeline = scrolls
        .iter()
        .find(|scroll| scroll.key == launcher::menu::studio::TIMELINE_SCROLL_ID)
        .expect("track viewport remains reachable")
        .clone();
    let row_action = MenuAction::Studio(Action::Timeline(TimelineAction::ToggleMute(
        Track::Video(0),
    )));
    let row = full
        .iter()
        .find(|(action, _)| *action == row_action)
        .unwrap()
        .1;
    assert!(
        timeline.viewport.height() + 0.1 >= row.height(),
        "one full lane at {size:?}"
    );
    assert_eq!(
        hits.iter()
            .find(|(action, _)| *action == row_action)
            .unwrap()
            .1,
        row
    );
    assert!(picture.max().y() < timeline.viewport.min().y());
    for action in [
        Action::TogglePlayback,
        Action::Step(-1),
        Action::Step(1),
        Action::Monitoring(MonitoringAction::EditSeek),
        Action::Monitoring(MonitoringAction::ApplySeek),
        Action::Monitoring(MonitoringAction::StepSpeed(-1)),
        Action::Monitoring(MonitoringAction::StepSpeed(1)),
        Action::Monitoring(MonitoringAction::ToggleLoop),
    ] {
        let action = MenuAction::Studio(action);
        let bounds = hits
            .iter()
            .find(|(found, _)| *found == action)
            .expect("full transport remains reachable")
            .1;
        assert_eq!(
            bounds,
            full.iter().find(|(found, _)| *found == action).unwrap().1
        );
        assert!(bounds.min().x() >= 0.0 && bounds.max().x() <= size[0]);
        assert!(bounds.min().y() >= 0.0 && bounds.max().y() <= size[1]);
    }
    timeline
}

#[test]
fn comfortable_large_timeline_keeps_a_meaningful_900_pixel_picture() {
    assert_paint(&fixture(1), [900.0, 600.0], 149.0);
}

#[test]
fn native_small_windows_keep_media_and_a_complete_lane_for_every_preference() {
    for density in [Density::Compact, Density::Comfortable] {
        for size in [
            TimelineSize::Small,
            TimelineSize::Medium,
            TimelineSize::Large,
        ] {
            let mut view = fixture(1);
            let studio = Arc::make_mut(&mut view.studio);
            studio.workspace.density = density;
            studio.workspace.timeline_size = size;
            assert_paint(&view, [640.0, 480.0], 119.0);
        }
    }
}

#[test]
fn tools_and_properties_do_not_replace_the_image_when_the_window_is_narrow() {
    for panel in [
        EditorPanel::Media,
        EditorPanel::Inspector,
        EditorPanel::Preview,
    ] {
        let mut view = fixture(1);
        let workspace = &mut Arc::make_mut(&mut view.studio).workspace;
        workspace.panel = panel;
        workspace.media_hidden = false;
        workspace.inspector_hidden = false;
        assert_paint(&view, [640.0, 480.0], 70.0);
    }
}

#[test]
fn extra_overlap_lanes_scroll_instead_of_taking_the_picture_height() {
    let short = assert_paint(&fixture(1), [900.0, 600.0], 149.0);
    let long = assert_paint(&fixture(30), [900.0, 600.0], 149.0);
    assert!(long.max > short.max && long.track.is_some());
    assert_eq!(short.viewport, long.viewport);
}

#[test]
fn wide_default_compact_preserves_requested_split_until_content_is_exhausted() {
    let mut view = fixture(30);
    let studio = Arc::make_mut(&mut view.studio);
    studio.workspace.density = Density::Compact;
    studio.workspace.timeline_size = TimelineSize::Small;
    assert_paint(&view, [1280.0, 720.0], 230.0);
}
