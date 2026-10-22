use super::*;

#[test]
fn recording_animates_received_actors_outside_the_live_animation_view() {
    let (assets, _) = super::super::culling_capacity_tests::animated_catalog();
    let mut stream = super::super::culling_capacity_tests::entity_world(&assets, [0.0, 0.0, -5.0]);
    let live = Some(actor_animation::ActorAnimationView {
        planes: [[0.0, 0.0, 1.0, -100.0]; 6],
        camera: [0.0; 3],
        player_distance: 100.0,
        entity_radius: 100.0,
    });
    let mut replay = ReplayActors::default();
    assert_eq!(replay.animation_view(live), live);
    stream.set_actor_animation_view(replay.animation_view(live));
    stream.advance_actor_interpolation_frame(1);
    let sample = |stream: &WorldStream, alpha| {
        let layers = stream
            .authority()
            .actor_render_frame(alpha)
            .layers_with_skin(1)
            .unwrap();
        let layer = &layers.render[0];
        if layer.geometry.is_some() {
            layer.pose.to_vec()
        } else {
            stream.authority().actor_rig(1).unwrap().current.to_vec()
        }
    };
    assert_eq!(sample(&stream, 0.25), sample(&stream, 0.75));
    replay.set_enabled(true);
    assert!(replay.animation_view(live).is_none());
    stream.set_actor_animation_view(replay.animation_view(live));
    stream.advance_actor_interpolation_frame(1);
    assert_ne!(sample(&stream, 0.25), sample(&stream, 0.75));
    replay.set_enabled(false);
    assert_eq!(replay.animation_view(live), live);
    stream.set_actor_animation_view(replay.animation_view(live));
    stream.advance_actor_interpolation_frame(1);
    assert_eq!(sample(&stream, 0.25), sample(&stream, 0.75));
}

#[test]
fn actor_preparation_switches_animation_visibility_with_recording_state() {
    use bevy::ecs::system::SystemState;
    use bevy::prelude::{PerspectiveProjection, Projection, Time, Transform, World};
    use bevy::time::Real;

    let (assets, artwork) = super::super::culling_capacity_tests::animated_catalog();
    let mut stream = super::super::culling_capacity_tests::entity_world(&assets, [0.0, 0.0, 50.0]);
    let mut world = World::new();
    world.init_resource::<Time<Real>>();
    world.insert_resource(render_scene::ActorArtworkResource(artwork));
    world.insert_resource(ActorRenderScene::with_runtime_entity_assets(&assets).unwrap());
    world.insert_resource(HandRigBuilder(ActorRigFrameBuilder::new([]).unwrap()));
    world.init_resource::<ActorFrameState>();
    world.init_resource::<PreparedActorPublication>();
    world.init_resource::<ActorFramePartialTick>();
    world.init_resource::<render_scene::HandRigScene>();
    let mut avatar = crate::local_player::LocalAvatarPresentation::default();
    avatar.begin_session(
        simulation_clock::identity::SessionGeneration::new(stream.authority().actor_session_id()),
        2,
    );
    world.insert_resource(avatar);
    world.init_resource::<crate::local_player::LocalAvatarVisibilityCarrier>();
    world.init_resource::<crate::camera::CameraSettingsAuthority>();
    world.init_resource::<crate::local_player::LocalViewPose>();
    world.spawn((
        Transform::default(),
        Projection::Perspective(PerspectiveProjection::default()),
        crate::camera::FlyCamera::default(),
    ));
    let mut prepared_artwork = None;
    for enabled in [None, Some(false), Some(true), Some(false)] {
        if let Some(enabled) = enabled {
            world.init_resource::<ReplayActors>();
            world.resource_mut::<ReplayActors>().set_enabled(enabled);
        }
        world
            .resource_mut::<Time<Real>>()
            .advance_by(simulation_clock::TICK_DURATION);
        let mut params = SystemState::<ActorFramePublication>::new(&mut world);
        advance_actor_frame(
            ActorWorld {
                stream: Some(&mut stream),
                collisions: None,
                entity_assets: Some(&assets),
                pack_entities: None,
                session_items: None,
                prepared_actor_artwork: &mut prepared_artwork,
            },
            ActorFrameInput {
                cosmetics: Vec::new(),
                remote_hats: Vec::new(),
                local_feed: None,
                predicted_eye: None,
                predicted_feet: None,
                local_equipment: Default::default(),
                swing_progress: None,
                renders_game: true,
                hide_hand: true,
                custom_emote: None,
                remote_emotes: Vec::new(),
            },
            |_| {},
            |_, _| (None, None),
            params.get_mut(&mut world).unwrap(),
        );
        let sample = |alpha| {
            let layers = stream
                .authority()
                .actor_render_frame(alpha)
                .layers_with_skin(1)
                .unwrap();
            assert!(!layers.render.is_empty());
            let layer = &layers.render[0];
            assert_eq!(layer.geometry.is_some(), enabled == Some(true));
            let pose = if layer.geometry.is_some() {
                layer.pose.to_vec()
            } else {
                stream.authority().actor_rig(1).unwrap().current.to_vec()
            };
            assert!(!pose.is_empty());
            pose
        };
        let changed = sample(0.25) != sample(0.75);
        assert_eq!(
            changed,
            enabled == Some(true),
            "recording state {enabled:?}"
        );
    }
}

#[test]
fn changing_replay_layer_poses_release_old_source_allocations() {
    let (assets, artwork) = super::super::culling_capacity_tests::animated_catalog();
    let stream = super::super::culling_capacity_tests::entity_world(&assets, [0.0, 0.0, -5.0]);
    let mut scene = ActorRenderScene::with_runtime_entity_assets(&assets).unwrap();
    scene.replace_pack_entities(Some(&assets)).unwrap();
    let mut replay = ReplayActors::default();
    let mut hand = ActorRigFrameBuilder::new(render_model::pack_geometries(&assets)).unwrap();
    let mut skins = crate::presentation::skin_rig::SkinRigCache::default();
    replay.set_enabled(true);
    let mut previous = None;
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
        let frame = &replay.frame.as_ref().unwrap().rig;
        assert!(
            !frame.manifest.is_empty(),
            "received draw rejection: {:?}",
            frame.rejects
        );
        if let Some(previous) = previous {
            assert_ne!(
                previous, frame.current_bones,
                "each frame must change its pose"
            );
        }
        previous = Some(Arc::clone(&frame.current_bones));
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
