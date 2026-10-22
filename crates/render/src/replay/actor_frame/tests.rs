use super::*;
use recording_studio::{camera::CameraPose, object_store::AssetId};
use render_model::{
    ActorDrawManifestEntry, ActorGpuInstance, ActorRenderIdentity, ActorRigRoute, EntityRigId,
};

/// Two layers of one spawn and another actor exercise packing independently of source bone bases.
fn source() -> FrozenActors {
    let identities = [(7, 0), (7, 1), (8, 0)];
    let instances = identities
        .iter()
        .map(|&(runtime_id, _)| ActorInstance {
            runtime_id,
            position: [runtime_id as f32, 0.0, 0.0],
            pitch_radians: 0.0,
            yaw_radians: 0.5,
            head_yaw_radians: 0.25,
            skin_layer: 0,
        })
        .collect::<Vec<_>>()
        .into();
    let manifest = identities
        .iter()
        .enumerate()
        .map(|(index, &(runtime_id, layer))| ActorDrawManifestEntry {
            identity: ActorRenderIdentity {
                session_id: 1,
                dimension: 0,
                runtime_id,
                spawn_revision: 1,
                ingress_sequence: 1,
                source_tick: Some(1),
                movement_revision: 1,
                pose_generation: render_api::PoseGeneration::new(1),
                layer,
            },
            rig: EntityRigId(0),
            completed_tick: 1,
            reset_generation: 0,
            route: ActorRigRoute::Compiled,
            instance_index: index as u32,
            previous_bone_base: index as u32,
            current_bone_base: index as u32,
            bone_count: 1,
        })
        .collect::<Vec<_>>()
        .into();
    let gpu = (0..identities.len())
        .map(|index| ActorGpuInstance {
            previous_bone_base: index as u32,
            current_bone_base: index as u32,
            ..Default::default()
        })
        .collect::<Vec<_>>()
        .into();
    FrozenActors {
        sample: ActorSample {
            coverage: recording_scene::actors::ActorCoverage::ReceivedWorld,
            instances,
            instance_revision: 2,
            skin_revision: 3,
            rig: ActorRigRenderFrame {
                frame_generation: 9,
                manifest,
                instances: gpu,
                previous_bones: vec![[[0.0; 4]; 3]; identities.len()].into(),
                current_bones: vec![[[1.0; 4]; 3]; identities.len()].into(),
                ..Default::default()
            },
            instance_pages: vec![1, 2, 3].into(),
        },
        skins: Arc::default(),
        artwork: Arc::default(),
        geometry_identity: AssetId([1; 32]),
        appearance_identity: AssetId([2; 32]),
    }
}

/// Recorded source body policy changes the effective mask without altering HUD or camera state.
fn visibility(hidden: bool) -> ActorVisibility {
    ActorVisibility::new(7, Some(!hidden), None, CameraPose::default(), true, None)
}

#[test]
fn paused_view_updates_keep_actor_generation_and_share_unmasked_rig_buffers() {
    let source = source();
    let mut publication = Publication::default();
    let mut generation = 0;
    let frame = publication
        .prepare(&source, visibility(false), false, &mut generation)
        .unwrap()
        .unwrap();
    assert!(Arc::ptr_eq(
        &frame.rig.instances,
        &source.sample.rig.instances
    ));
    assert!(Arc::ptr_eq(
        &frame.rig.manifest,
        &source.sample.rig.manifest
    ));
    assert!(Arc::ptr_eq(
        &frame.instance_pages,
        &source.sample.instance_pages
    ));
    for yaw in [0.0, 90.0, 270.0] {
        let view = ActorVisibility::new(
            7,
            None,
            None,
            CameraPose {
                position: [20.0, 30.0, 40.0],
                yaw,
                fov: 110.0,
                ..Default::default()
            },
            false,
            None,
        );
        assert!(
            publication
                .prepare(&source, view, false, &mut generation)
                .unwrap()
                .is_none()
        );
    }
    assert_eq!(generation, frame.rig.frame_generation);
}

#[test]
fn unchanged_publications_do_not_allocate_or_advance_gpu_generation() {
    let source = source();
    let mut publication = Publication::default();
    let mut generation = 0;
    publication
        .prepare(&source, visibility(false), false, &mut generation)
        .unwrap();
    let (_, allocations) = test_alloc::measure(|| {
        for _ in 0..64 {
            assert!(
                publication
                    .prepare(&source, visibility(false), false, &mut generation)
                    .unwrap()
                    .is_none()
            );
        }
    });
    assert_eq!(allocations.calls, 0);
    assert_eq!(generation, 1);
}

#[test]
fn body_mask_changes_publish_once_and_keep_remaining_source_bone_addresses() {
    let source = source();
    let mut publication = Publication::default();
    let mut generation = 0;
    publication
        .prepare(&source, visibility(false), false, &mut generation)
        .unwrap();
    let hidden = publication
        .prepare(&source, visibility(true), false, &mut generation)
        .unwrap()
        .unwrap();
    assert_eq!(hidden.instances.len(), 1);
    assert_eq!(hidden.instances[0].runtime_id, 8);
    assert_eq!(hidden.instance_pages.as_ref(), &[3]);
    assert_eq!(hidden.rig.manifest[0].instance_index, 0);
    assert_eq!(hidden.rig.manifest[0].current_bone_base, 2);
    assert!(Arc::ptr_eq(
        &hidden.rig.current_bones,
        &source.sample.rig.current_bones
    ));
    assert!(
        publication
            .prepare(&source, visibility(true), false, &mut generation)
            .unwrap()
            .is_none()
    );
    let restored = publication
        .prepare(&source, visibility(false), false, &mut generation)
        .unwrap()
        .unwrap();
    assert_eq!(restored.instances.len(), source.sample.instances.len());
    assert!(Arc::ptr_eq(
        &restored.rig.instances,
        &source.sample.rig.instances
    ));
    assert!(restored.rig.frame_generation > hidden.rig.frame_generation);
}

#[test]
fn source_metadata_resources_and_interpolated_buffers_cannot_reuse_a_stale_publication() {
    let changes: &[fn(&mut FrozenActors)] = &[
        |s| s.sample.instance_revision += 1,
        |s| s.sample.skin_revision += 1,
        |s| s.sample.rig.frame_generation += 1,
        |s| s.sample.rig.geometry_revision += 1,
        |s| s.sample.rig.maximum_vertex_count += 1,
        |s| s.sample.rig.rejects.invalid_identity += 1,
        |s| s.geometry_identity.0[0] += 1,
        |s| s.appearance_identity.0[0] += 1,
        |s| s.skins = Arc::default(),
        |s| s.artwork = Arc::default(),
        |s| Arc::make_mut(&mut s.sample.instances)[0].position[0] += 1.0,
        |s| Arc::make_mut(&mut s.sample.rig.instances)[0].partial_tick = 0.5,
        |s| {
            Arc::make_mut(&mut s.sample.rig.manifest)[0]
                .identity
                .spawn_revision += 1
        },
        |s| Arc::make_mut(&mut s.sample.rig.previous_bones)[0][0][0] += 1.0,
        |s| Arc::make_mut(&mut s.sample.rig.current_bones)[0][0][0] += 1.0,
        |s| Arc::make_mut(&mut s.sample.instance_pages)[0] += 1,
        |s| s.sample.rig.geometry_vertices.epoch += 1,
        |s| s.sample.rig.geometry_spans = Arc::from([]),
    ];
    for change in changes {
        let source = source();
        let mut publication = Publication::default();
        let mut generation = 0;
        publication
            .prepare(&source, visibility(false), false, &mut generation)
            .unwrap();
        let mut next = source.clone();
        change(&mut next);
        assert!(
            publication
                .prepare(&next, visibility(false), false, &mut generation)
                .unwrap()
                .is_some()
        );
        assert_eq!(generation, 2);
    }
}

#[test]
fn temporal_reset_forces_a_new_publication_and_generation_overflow_is_transactional() {
    let source = source();
    let mut publication = Publication::default();
    let mut generation = 0;
    publication
        .prepare(&source, visibility(false), false, &mut generation)
        .unwrap();
    assert!(
        publication
            .prepare(&source, visibility(false), true, &mut generation)
            .unwrap()
            .is_some()
    );
    let at_reset = generation;
    generation = u64::MAX;
    assert!(
        publication
            .prepare(&source, visibility(true), false, &mut generation)
            .is_err()
    );
    assert_eq!(generation, u64::MAX);
    generation = at_reset;
    assert!(
        publication
            .prepare(&source, visibility(false), false, &mut generation)
            .unwrap()
            .is_none()
    );
}

#[test]
fn replacing_or_dropping_a_publication_releases_prior_input_roots() {
    let source = source();
    let weak = Arc::downgrade(&source.sample.instances);
    let mut publication = Publication::default();
    let mut generation = 0;
    let first = publication
        .prepare(&source, visibility(false), false, &mut generation)
        .unwrap()
        .unwrap();
    drop(source);
    assert!(weak.upgrade().is_some());
    let next = self::source();
    publication
        .prepare(&next, visibility(false), false, &mut generation)
        .unwrap();
    assert!(weak.upgrade().is_none());
    assert_eq!(
        first.instances.len(),
        3,
        "published render data retains independent ownership"
    );
    let weak = Arc::downgrade(&next.sample.instances);
    drop(next);
    drop(publication);
    assert!(weak.upgrade().is_none());
}
