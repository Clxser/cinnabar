use super::*;

/// Both selections use one ordinary source of actor eligibility, names, settings and prepared text.
fn project(cache: &mut NametagTextCache, record: bool) -> Vec<NametagAnchor> {
    let stream = super::super::tests::actor_stream(6, None, 0);
    cache.request_recording(record);
    project_nametags(
        cache,
        &ui::ScoreboardStore::default(),
        &stream,
        NametagView {
            camera_position: Vec3::ZERO,
            camera_forward: Vec3::NEG_Z,
        },
        None,
        1.0,
        true,
        |anchor| {
            anchor.set_cinnabar_badge(true);
            anchor.runtime_id() % 2 == 0
        },
    )
}

#[test]
fn offscreen_labels_are_recorded_without_changing_the_live_selection_or_decorations() {
    let mut cache = NametagTextCache::default();
    let visible = project(&mut cache, true);
    assert_eq!(visible.len(), 3);
    assert_eq!(cache.recording.anchors.len(), 6);
    assert!(
        cache
            .recording
            .anchors
            .iter()
            .all(|anchor| anchor.cinnabar_badge)
    );
    for live in &visible {
        let recorded = cache
            .recording
            .anchors
            .iter()
            .find(|tag| tag.runtime_id == live.runtime_id)
            .unwrap();
        assert_eq!(live, recorded);
        assert!(Arc::ptr_eq(&live.lines, &recorded.lines));
    }
    assert_eq!(
        cache.entries.len(),
        6,
        "each actor has one shared text extraction"
    );
    assert_eq!(visible, project(&mut cache, false));
    assert!(cache.recording.anchors.is_empty());
}

#[test]
fn turning_the_original_camera_cannot_erase_eligible_recording_labels() {
    let stream = super::super::tests::actor_stream(6, None, 0);
    let mut presentation = crate::test_support::mini_engine_presentation();
    let runtime = crate::ui_runtime::UiRuntime::new(1);
    let mut cache = NametagTextCache::default();
    let camera = Vec3::new(2.5, DEFAULT_HEIGHT + HEAD_CLEARANCE, 1.0);
    let boards = ui::ScoreboardStore::default();
    for rotation in [
        glam::Quat::IDENTITY,
        glam::Quat::from_rotation_y(std::f32::consts::PI),
    ] {
        presentation.prepare_nametag_layout(&runtime);
        let clip = glam::Mat4::perspective_rh(70_f32.to_radians(), 1.0, 0.1, 100.0)
            * glam::Mat4::from_rotation_translation(rotation, camera).inverse();
        cache.request_recording(true);
        let live = project_nametags(
            &mut cache,
            &boards,
            &stream,
            NametagView {
                camera_position: camera,
                camera_forward: rotation * Vec3::NEG_Z,
            },
            None,
            1.0,
            true,
            |tag| presentation.nametag_intersects_view(tag, clip, rotation),
        );
        assert!(
            live.len() < 6,
            "the original frustum omits at least one eligible label"
        );
        assert_eq!(cache.recording.anchors.len(), 6);
        cache.recycle_anchors(live);
    }
}

#[test]
fn recording_uses_the_existing_label_preparation_budget() {
    let stream = super::super::tests::actor_stream(MAX_PRESENTED_NAMETAGS * 3, None, 0);
    let mut cache = NametagTextCache::default();
    cache.request_recording(true);
    let live = project_nametags(
        &mut cache,
        &ui::ScoreboardStore::default(),
        &stream,
        NametagView {
            camera_position: Vec3::ZERO,
            camera_forward: Vec3::NEG_Z,
        },
        None,
        1.0,
        true,
        |tag| tag.runtime_id() % 2 == 0,
    );
    assert_eq!(live.len(), MAX_PRESENTED_NAMETAGS);
    assert_eq!(cache.recording.anchors.len(), MAX_PRESENTED_NAMETAGS);
}

#[test]
fn capture_cadence_retains_an_independent_atlas_and_disabling_releases_it() {
    let mut presentation = crate::test_support::mini_engine_presentation();
    let mut cache = NametagTextCache::default();
    presentation.request_nametag_recording(true, true);
    let visible = project(&mut cache, true);
    cache.publish_recording(&mut presentation, 0);
    let recorded = Arc::clone(presentation.recorded_nametags().unwrap());
    assert_eq!(recorded.labels.len(), 6);
    presentation.set_nametag_anchors(visible);
    let live = presentation.nametag_scene();
    assert_eq!(live.labels.len(), 3);
    assert!(!Arc::ptr_eq(&live.atlas, &recorded.atlas));
    let artwork = recorded.atlas.clone();
    presentation.set_nametag_anchors(vec![]);
    let _ = presentation.nametag_scene();
    assert_eq!(recorded.atlas, artwork);
    presentation.request_nametag_recording(true, false);
    assert!(!presentation.nametag_recording_requested());
    assert!(Arc::ptr_eq(
        presentation.recorded_nametags().unwrap(),
        &recorded
    ));
    presentation.request_nametag_recording(false, false);
    assert!(presentation.recorded_nametags().is_none());
}

#[test]
fn unchanged_capture_selection_reuses_the_text_and_anchor_storage() {
    let stream = super::super::tests::actor_stream(6, None, 0);
    let mut cache = NametagTextCache::default();
    let boards = ui::ScoreboardStore::default();
    let sample = |cache: &mut NametagTextCache| {
        cache.request_recording(true);
        let visible = project_nametags(
            cache,
            &boards,
            &stream,
            NametagView {
                camera_position: Vec3::ZERO,
                camera_forward: Vec3::NEG_Z,
            },
            None,
            1.0,
            true,
            |_| false,
        );
        cache.recycle_anchors(visible);
    };
    sample(&mut cache);
    let (_, allocations) = test_alloc::count(|| sample(&mut cache));
    assert_eq!(allocations, 0);
    assert_eq!(cache.recording.anchors.len(), 6);
}

#[test]
fn settings_filter_and_session_font_reset_apply_to_recording_too() {
    let mut cache = NametagTextCache::default();
    let _ = project(&mut cache, true);
    cache.filter_recorded(|anchor| anchor.runtime_id() % 2 == 1);
    let mut presentation = crate::test_support::mini_engine_presentation();
    presentation.request_nametag_recording(true, true);
    cache.publish_recording(&mut presentation, 0);
    assert_eq!(presentation.recorded_nametags().unwrap().labels.len(), 3);
    presentation.nametag_recording.reset_artwork();
    assert!(presentation.recorded_nametags().is_none());
    assert!(presentation.nametag_recording_requested());
}
