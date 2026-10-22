use super::*;

/// Uses the ordinary received actor and local publication owners without a renderer or live server.
fn world(remote_flags: u64) -> World {
    let mut stream = chunk_pipeline::WorldStream::new_with_assets(
        protocol::WorldBootstrap {
            dimension: 0,
            local_player_runtime_id: 7,
            local_player_unique_id: 70,
            player_position: [0.0; 3],
            world_spawn_position: [0; 3],
            air_network_id: protocol::air_network_id(false),
            block_network_ids_are_hashes: false,
        },
        Arc::new(assets::RuntimeAssets::diagnostic()),
        [0.0; 3],
        None,
    );
    for (index, (runtime_id, unique_id, flags)) in
        [(7, 70, 0), (8, 80, remote_flags)].into_iter().enumerate()
    {
        stream
            .submit(
                index as u64 + 1,
                protocol::WorldEvent::Actor(protocol::ActorEvent::Spawn(
                    protocol::ActorSpawnEvent {
                        dimension: 0,
                        runtime_id,
                        unique_id,
                        kind: protocol::ActorKind::Player {
                            uuid: [runtime_id as u8; 16],
                            username: "Subject".into(),
                        },
                        position: [runtime_id as f32, 30.0, 5.0],
                        velocity: [0.0; 3],
                        pitch: 0.0,
                        yaw: 0.0,
                        head_yaw: 0.0,
                        body_yaw: 0.0,
                        held_item: Default::default(),
                        metadata: Arc::from([protocol::ActorMetadata {
                            key: protocol_ids::ActorDataKey::Flags.wire_value(),
                            value: protocol::ActorMetadataValue::Flags(flags),
                        }]),
                        attributes: Arc::from([]),
                        properties: Arc::from([]),
                        links: Arc::from([]),
                    },
                )),
            )
            .unwrap();
    }
    stream.poll([0.0; 3], 0);
    let mut world = World::new();
    world.insert_resource(ClientWorld {
        stream: Some(stream),
        ..Default::default()
    });
    world
}

/// Camera transform, collision feet and exact published eye intentionally have different origins.
fn local_carrier(session: u64) -> LocalPlayerFrameCarrier {
    let mut carrier = LocalPlayerFrameCarrier::default();
    carrier
        .publish(client_presentation::local_player::LocalPlayerFrameSample {
            session_generation: simulation_clock::identity::SessionGeneration::new(1),
            actor_session_id: session,
            fifo_sequence: protocol::identity::FifoSequence::new(1),
            physics_tick: simulation_clock::PhysicsTick::new(1),
            perspective: semantic_input::PerspectiveMode::ThirdPersonBack,
            world_collision_identity: sim::WorldCollisionIdentity {
                registry: sim::CollisionRegistryIdentity {
                    protocol: protocol::PROTOCOL_VERSION as u32,
                    id_space: sim::CollisionIdSpace::Sequential,
                    preg_sha256: [0; 32],
                },
                chunks: Box::new([]),
            },
            pose: Transform::from_xyz(40.0, 80.0, 60.0),
            feet: Vec3::new(1.0, 2.0, 3.0),
            eye: Vec3::new(1.25, 2.4, 3.5),
            rotation: Quat::from_rotation_y(0.25),
        })
        .unwrap();
    carrier
}

#[test]
fn missing_or_stale_local_eye_keeps_received_subject_lifetime_for_centre_cameras() {
    let mut world = world(0);
    let mut targets = Vec::new();
    snapshot(&world, &mut targets).unwrap();
    let local = targets
        .iter()
        .find(|target| target.runtime_id == 7)
        .unwrap()
        .clone();
    assert_eq!(local.unique_id, 70);
    assert!(local.spawn_revision > 0);
    assert_eq!(local.view, CameraTargetView::MissingLocalPresentation);
    assert!(local.pose.eye_offset.is_none());
    assert!(local.validate().is_ok());
    let authority = world
        .resource::<ClientWorld>()
        .stream
        .as_ref()
        .unwrap()
        .authority();
    assert_eq!(
        local.pose.position,
        authority
            .actor(7)
            .unwrap()
            .interpolated_position(0.0)
            .unwrap()
            .map(f64::from)
    );
    let session = authority.actor_session_id();
    world.insert_resource(local_carrier(session + 1));
    snapshot(&world, &mut targets).unwrap();
    assert_eq!(
        targets
            .iter()
            .find(|target| target.runtime_id == 7)
            .unwrap(),
        &local
    );
}

#[test]
fn local_pose_uses_exact_published_eye_even_when_third_person_camera_is_elsewhere() {
    let mut world = world(0);
    let session = world
        .resource::<ClientWorld>()
        .stream
        .as_ref()
        .unwrap()
        .authority()
        .actor_session_id();
    let carrier = local_carrier(session);
    let sample = carrier.snapshot().unwrap().clone();
    world.insert_resource(carrier);
    let mut targets = Vec::new();
    snapshot(&world, &mut targets).unwrap();
    let local = targets
        .iter()
        .find(|target| target.runtime_id == 7)
        .unwrap();
    assert_eq!(local.view, CameraTargetView::LocalPresentation);
    assert_eq!(local.pose.position, sample.feet().to_array().map(f64::from));
    assert_eq!(
        local.pose.eye_offset,
        Some((sample.eye() - sample.feet()).to_array().map(f64::from))
    );
    assert_ne!(
        local.pose.position,
        sample.pose().translation.to_array().map(f64::from)
    );
}

#[test]
fn remote_prone_pose_retains_body_target_and_precise_unsupported_eye_evidence() {
    let world = world(1_u64 << protocol_ids::ActorFlag::Swimming.wire_value());
    let mut targets = Vec::new();
    snapshot(&world, &mut targets).unwrap();
    let remote = targets
        .iter()
        .find(|target| target.runtime_id == 8)
        .unwrap();
    assert_eq!(remote.view, CameraTargetView::Swimming);
    assert!(remote.pose.eye_offset.is_none());
    assert_eq!(
        remote.eye_view_notice(),
        Some(CameraTargetView::Swimming.notice())
    );
    assert!(remote.validate().is_ok());
}
