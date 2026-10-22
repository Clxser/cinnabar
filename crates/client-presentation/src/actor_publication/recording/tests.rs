use super::*;

#[test]
fn changing_replay_layer_poses_release_old_source_allocations() {
    let (assets, artwork) = super::super::culling_capacity_tests::catalog(
        None,
        true,
        "query.frame_alpha > 0.5 ? Geometry.wide : Geometry.default",
    );
    let stream = super::super::culling_capacity_tests::entity_world(&assets, [0.0, 0.0, -5.0]);
    let mut scene = ActorRenderScene::with_runtime_entity_assets(&assets).unwrap();
    let mut replay = ReplayActors::default();
    let mut hand = ActorRigFrameBuilder::new(render_model::pack_geometries(&assets)).unwrap();
    let mut skins = crate::presentation::skin_rig::SkinRigCache::default();
    replay.set_enabled(true);
    for index in 1..=64 {
        let alpha = index as f32 / 64.0;
        replay.request_frame();
        prepare(
            &mut replay,
            Some(&stream),
            &ActorFrameInput {
                cosmetics: Vec::new(),
                remote_hats: Vec::new(),
                local_feed: None,
                predicted_eye: None,
                predicted_feet: None,
                local_equipment: Default::default(),
                swing_progress: None,
                renders_game: true,
                hide_hand: false,
                custom_emote: None,
                remote_emotes: Vec::new(),
            },
            bodies::PoseView {
                partial_tick: alpha,
                first_person: true,
                java_mode: false,
                combat_visuals: Default::default(),
                sampling_camera: None,
                aspect: 1.0,
            },
            None,
            &artwork,
            Some(&assets),
            None,
            None,
            &mut skins,
            &mut scene,
            &mut hand,
            None,
            None,
            Some(stream.authority().actor_render_frame(alpha)),
        );
        assert!(!replay.frame.as_ref().unwrap().rig.manifest.is_empty());
    }
    let retained = replay.scratch.layer_poses.retained_pose_count();
    assert!(
        retained > 0,
        "fixture must exercise native layer conversions"
    );
    assert!(
        retained < 32,
        "old source poses must retire, retained {retained}"
    );
}

#[test]
fn received_publication_keeps_live_frame_artwork_and_geometry_leases() {
    let mut scene = ActorRenderScene::default();
    scene.update(0.5, None, []);
    let live = scene.frame().clone();
    let mut replay = ReplayActors::default();
    replay.set_enabled(true);
    replay.request_frame();
    let mut hand = ActorRigFrameBuilder::new([]).unwrap();
    let mut skins = crate::presentation::skin_rig::SkinRigCache::default();
    prepare(
        &mut replay,
        None,
        &ActorFrameInput {
            cosmetics: Vec::new(),
            remote_hats: Vec::new(),
            local_feed: None,
            predicted_eye: None,
            predicted_feet: None,
            local_equipment: Default::default(),
            swing_progress: None,
            renders_game: true,
            hide_hand: false,
            custom_emote: None,
            remote_emotes: Vec::new(),
        },
        bodies::PoseView {
            partial_tick: 0.5,
            first_person: true,
            java_mode: false,
            combat_visuals: render_api::CombatVisuals::Modern,
            sampling_camera: None,
            aspect: 1.0,
        },
        None,
        &render_model::ActorArtworkPages::default(),
        None,
        None,
        None,
        &mut skins,
        &mut scene,
        &mut hand,
        None,
        None,
        None,
    );
    assert!(replay.frame.is_some());
    assert_eq!(
        scene.frame().rig.frame_generation,
        live.rig.frame_generation
    );
    assert!(Arc::ptr_eq(&scene.frame().artwork, &live.artwork));
    assert!(Arc::ptr_eq(
        &scene.frame().rig.instances,
        &live.rig.instances
    ));
    assert!(Arc::ptr_eq(
        &scene.frame().rig.geometry_vertices.segments,
        &live.rig.geometry_vertices.segments
    ));
    assert!(Arc::ptr_eq(
        &replay
            .frame
            .as_ref()
            .unwrap()
            .rig
            .geometry_vertices
            .segments,
        &live.rig.geometry_vertices.segments
    ));
}
