use super::*;

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
