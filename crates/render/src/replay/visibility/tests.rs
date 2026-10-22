use super::*;
use recording_scene::{
    actors::{ActorCoverage, ActorInstance, ActorSample, FrozenActors},
    targets::TargetFrame,
};
use recording_studio::{
    Id, MediaTime, Span,
    camera::{
        ActorPose, CameraPose, CameraRigKind, CameraShot, CameraTarget, CameraTargetKind,
        TargetLifetime,
    },
    object_store::AssetId,
};
use render_model::{
    ActorDrawManifestEntry, ActorGpuInstance, ActorRigGeometry, ActorRigRenderFrame, ActorRigRoute,
    ActorRigVertexSegments, EntityRigId,
};
use std::sync::Arc;

const MATRIX: [[f32; 4]; 3] = [[1., 0., 0., 0.], [0., 1., 0., 0.], [0., 0., 1., 0.]];
const SIZE: [u32; 2] = [800, 600];

/// Every identity is original fixture data with an explicit source session and spawn lifetime.
fn identity(runtime: u64, spawn: u64, layer: u8) -> ActorRenderIdentity {
    ActorRenderIdentity {
        session_id: 1,
        dimension: 0,
        runtime_id: runtime,
        spawn_revision: spawn,
        ingress_sequence: 1,
        source_tick: None,
        movement_revision: 1,
        pose_generation: render_api::PoseGeneration::new(1),
        layer,
    }
}

/// One captured cube and one captured matrix per independently identified draw layer.
fn rig(draws: &[(u64, u64, u8, [f32; 3])], min: [f32; 3], max: [f32; 3]) -> ActorRigRenderFrame {
    let geometry = ActorRigGeometry::synthetic_cuboid(EntityRigId(1), min, max, 1).unwrap();
    let manifest = draws
        .iter()
        .enumerate()
        .map(
            |(index, &(runtime, spawn, layer, _))| ActorDrawManifestEntry {
                identity: identity(runtime, spawn, layer),
                rig: geometry.id,
                completed_tick: 1,
                reset_generation: 1,
                route: ActorRigRoute::Compiled,
                instance_index: index as u32,
                previous_bone_base: index as u32,
                current_bone_base: index as u32,
                bone_count: 1,
            },
        )
        .collect();
    let instances = draws
        .iter()
        .enumerate()
        .map(|(index, &(_, _, _, position))| {
            let mut world = MATRIX;
            for axis in 0..3 {
                world[axis][3] = position[axis];
            }
            ActorGpuInstance {
                world_from_actor: world,
                previous_bone_base: index as u32,
                current_bone_base: index as u32,
                geometry_id: 0,
                partial_tick: 1.,
                ..Default::default()
            }
        })
        .collect();
    ActorRigRenderFrame {
        frame_generation: 1,
        geometry_revision: 1,
        instances,
        manifest,
        previous_bones: vec![MATRIX; draws.len()].into(),
        current_bones: vec![MATRIX; draws.len()].into(),
        geometry_spans: Arc::from([render_model::ActorRigGeometrySpan {
            first_vertex: 0,
            vertex_count: geometry.vertices.len() as u32,
        }]),
        geometry_vertices: ActorRigVertexSegments::from_vertices(Arc::clone(&geometry.vertices)),
        maximum_vertex_count: geometry.vertices.len() as u32,
        ..Default::default()
    }
}

/// The eye is deliberately independent of target metadata; the mesh supplies collision geometry.
fn camera(position: [f64; 3]) -> CameraPose {
    CameraPose {
        position,
        ..Default::default()
    }
}

/// An admitted local target keeps source/spectated policy separate from captured rig geometry.
fn target() -> CameraTarget {
    CameraTarget {
        runtime_id: 7,
        unique_id: 10,
        spawn_revision: 3,
        name: Arc::from("Fixture"),
        kind: CameraTargetKind::Player,
        entity_type: Some(Arc::from("minecraft:player")),
        teleported: false,
        pose: ActorPose {
            position: [0.; 3],
            velocity: [0.; 3],
            yaw: 0.,
            pitch: 0.,
            roll: 0.,
            eye_offset: None,
        },
    }
}

/// The complete sampled body source also exercises the real actor-publication cache.
fn frozen(rig: ActorRigRenderFrame) -> FrozenActors {
    let instances = rig
        .manifest
        .iter()
        .map(|entry| ActorInstance {
            runtime_id: entry.identity.runtime_id,
            position: [0.; 3],
            pitch_radians: 0.,
            yaw_radians: 0.,
            head_yaw_radians: 0.,
            skin_layer: 0,
        })
        .collect();
    let pages = vec![0; rig.instances.len()].into();
    FrozenActors {
        sample: ActorSample {
            coverage: ActorCoverage::ReceivedWorld,
            instances,
            instance_revision: 1,
            skin_revision: 1,
            rig,
            instance_pages: pages,
        },
        skins: Arc::default(),
        artwork: Arc::default(),
        geometry_identity: AssetId([1; 32]),
        appearance_identity: AssetId([2; 32]),
    }
}

#[test]
fn captured_geometry_masks_only_the_intersecting_layer_and_exact_lifetime() {
    let body = identity(7, 3, render_model::ACTOR_LAYER_BODY);
    let equipment = identity(7, 3, 1);
    let other = identity(8, 3, 0);
    let rig = rig(
        &[
            (7, 3, 0, [0.; 3]),
            (7, 3, 1, [2., 0., 0.]),
            (8, 3, 0, [3., 0., 0.]),
        ],
        [-0.25; 3],
        [0.25; 3],
    );
    let mut cache = IntersectionCache::default();
    let pose = camera([0.; 3]);
    let mask = cache.mask(&rig, pose, SIZE);
    assert_eq!(mask, [DrawKey::from(&body)]);
    let visibility =
        ActorVisibility::new(7, Some(false), None, false, None).with_intersections(mask);
    assert!(!visibility.visible(&body));
    assert!(visibility.visible(&equipment));
    assert!(visibility.visible(&other));
    for (position, intersects) in [
        ([2., 2., 2.], false),
        ([0.; 3], true),
        ([2., 2., 2.], false),
        ([0.; 3], true),
    ] {
        assert_eq!(
            !cache.mask(&rig, camera(position), SIZE).is_empty(),
            intersects,
            "camera history and direction of travel do not affect the geometric result"
        );
    }
    for changed in [
        ActorRenderIdentity {
            spawn_revision: 4,
            ..body
        },
        ActorRenderIdentity {
            session_id: 2,
            ..body
        },
        ActorRenderIdentity {
            dimension: 1,
            ..body
        },
    ] {
        assert!(visibility.visible(&changed));
    }
    assert_eq!(
        pose,
        camera([0.; 3]),
        "masking does not reposition the authored camera"
    );
}

#[test]
fn actual_persona_extent_replaces_the_eye_radius_and_legacy_eye_requirement() {
    let rig = rig(&[(7, 3, 0, [0.; 3])], [2.5, -0.5, -0.5], [3.5, 0.5, 0.5]);
    let mut cache = IntersectionCache::default();
    assert_eq!(
        cache.mask(&rig, camera([3., 0., 0.]), SIZE),
        [DrawKey::from(&identity(7, 3, 0))]
    );
    assert!(
        cache.mask(&rig, camera([0.; 3]), SIZE).is_empty(),
        "no mesh occupies the old synthetic eye guard"
    );
    let target = target();
    assert!(target.pose.eye_offset.is_none());
    let targets = TargetFrame {
        at: MediaTime::ZERO,
        targets: vec![target].into(),
    };
    let visible = ActorVisibility::new(7, Some(false), Some(&targets), false, None)
        .with_intersections(cache.mask(&rig, camera([3., 0., 0.]), SIZE));
    assert!(
        !visible.visible(&identity(7, 3, 0)),
        "captured geometry works with legacy missing eye metadata"
    );
    assert!(
        cache
            .mask(&ActorRigRenderFrame::default(), camera([3., 0., 0.]), SIZE)
            .is_empty(),
        "missing geometry is not replaced by a guessed body"
    );
}

#[test]
fn a_free_camera_created_at_the_native_eye_keeps_its_pose_and_cannot_reveal_its_face() {
    let mut target = target();
    target.pose.position = [0.5, -60., 0.5];
    target.pose.eye_offset = Some([0., 1.62, 0.]);
    let rig = rig(
        &[(7, 3, 0, [0.5, -60., 0.5])],
        [-0.25, 1.4, -0.25],
        [0.25, 1.9, 0.25],
    );
    let mut cache = IntersectionCache::default();
    for (yaw, pitch, roll, fov) in [(0., 0., 0., 66.), (180., 80., 25., 120.)] {
        let eye = CameraPose {
            position: target.pose.centre(),
            yaw,
            pitch,
            roll,
            fov,
        };
        let shot = CameraShot::from_preview(
            Id(1),
            Span::new(MediaTime::ZERO, MediaTime(1_000_000)).unwrap(),
            CameraRigKind::Free,
            eye,
            None,
        )
        .unwrap();
        let pose = shot.sample(MediaTime::ZERO, |_| None).unwrap();
        assert_eq!(pose, eye);
        assert_eq!(
            cache.mask(&rig, pose, SIZE),
            [DrawKey::from(&identity(7, 3, 0))]
        );
    }
    assert!(
        cache.mask(&rig, camera([3., -58.38, 0.5]), SIZE).is_empty(),
        "flying away restores the captured body"
    );
}

#[test]
fn animation_partial_tick_and_world_scale_posture_rebind_without_rescanning_vertices() {
    let mut rig = rig(&[(7, 3, 0, [10., -7., 4.])], [-0.25; 3], [0.25; 3]);
    let instance = &mut Arc::make_mut(&mut rig.instances)[0];
    instance.world_from_actor[0][0] = 3.;
    instance.world_from_actor[1][1] = 0.25;
    instance.world_from_actor[2][2] = 2.;
    Arc::make_mut(&mut rig.previous_bones)[0][1][3] = 1.;
    Arc::make_mut(&mut rig.current_bones)[0][1][3] = 3.;
    Arc::make_mut(&mut rig.instances)[0].partial_tick = 0.5;
    let mut cache = IntersectionCache::default();
    let pose = camera([10., -6.5, 4.]);
    assert_eq!(
        cache.mask(&rig, pose, SIZE).len(),
        1,
        "mounted placement and crouched scale use the captured matrices"
    );
    let vertices = cache.geometry_vertices;
    for (partial, y) in [(0., -6.75), (1., -6.25), (0.5, -6.5)] {
        Arc::make_mut(&mut rig.instances)[0].partial_tick = partial;
        assert_eq!(cache.mask(&rig, camera([10., y, 4.]), SIZE).len(), 1);
        assert!(cache.mask(&rig, camera([0.; 3]), SIZE).is_empty());
    }
    assert_eq!(cache.geometry_vertices, vertices);
    let builds = cache.pose_builds;
    Arc::make_mut(&mut rig.current_bones)[0][0][3] = 10.;
    assert!(
        cache.mask(&rig, pose, SIZE).is_empty(),
        "changed bone allocation invalidates even unchanged generation numbers"
    );
    assert_eq!(cache.pose_builds, builds + 1);
    assert_eq!(cache.geometry_vertices, vertices);
}

#[test]
fn near_plane_not_nearby_distance_controls_clipping_and_lens_rotation() {
    let near = render_api::CAMERA_NEAR_PLANE_BLOCKS;
    let inside = rig(
        &[(7, 3, 0, [0.; 3])],
        [-0.001, -0.001, near * 0.75],
        [0.001, 0.001, near * 1.25],
    );
    let beyond = rig(
        &[(7, 3, 0, [0.; 3])],
        [-0.001, -0.001, near * 1.1],
        [0.001, 0.001, near * 1.5],
    );
    let touching = rig(
        &[(7, 3, 0, [0.; 3])],
        [-0.001, -0.001, near],
        [0.001, 0.001, near * 1.5],
    );
    let behind = rig(
        &[(7, 3, 0, [0.; 3])],
        [-0.001, -0.001, -near * 0.9],
        [0.001, 0.001, -near * 0.5],
    );
    let mut cache = IntersectionCache::default();
    let pose = camera([0.; 3]);
    assert_eq!(cache.mask(&inside, pose, SIZE).len(), 1);
    assert_eq!(cache.mask(&touching, pose, SIZE).len(), 1);
    assert!(cache.mask(&beyond, pose, SIZE).is_empty());
    assert!(cache.mask(&behind, pose, SIZE).is_empty());
    assert_eq!(
        cache
            .mask(&behind, CameraPose { yaw: 180., ..pose }, SIZE)
            .len(),
        1
    );
    let side = rig(
        &[(7, 3, 0, [0.; 3])],
        [near * 1.2, -0.001, near * 0.9],
        [near * 1.3, 0.001, near],
    );
    assert_eq!(
        cache
            .mask(&side, CameraPose { fov: 90., ..pose }, [800, 400])
            .len(),
        1
    );
    assert!(
        cache
            .mask(
                &side,
                CameraPose {
                    fov: 90.,
                    roll: 90.,
                    ..pose
                },
                [800, 400]
            )
            .is_empty()
    );
    assert!(
        cache
            .mask(&side, CameraPose { fov: 20., ..pose }, [800, 400])
            .is_empty()
    );
}

#[test]
fn rotated_boxes_reject_empty_world_aabb_corners_and_zero_scale() {
    let mut rig = rig(
        &[(7, 3, 0, [-0.2, 0.2, 0.01])],
        [-1., -0.01, -0.01],
        [1., 0.01, 0.01],
    );
    let (s, c) = std::f32::consts::FRAC_PI_4.sin_cos();
    Arc::make_mut(&mut rig.instances)[0].world_from_actor =
        [[c, -s, 0., -0.2], [s, c, 0., 0.2], [0., 0., 1., 0.01]];
    let mut cache = IntersectionCache::default();
    assert!(
        cache.mask(&rig, camera([0.; 3]), SIZE).is_empty(),
        "SAT does not mistake the surrounding AABB's empty corners for skin"
    );
    Arc::make_mut(&mut rig.current_bones)[0] = [[0.; 4]; 3];
    Arc::make_mut(&mut rig.instances)[0].world_from_actor = MATRIX;
    assert!(
        cache.mask(&rig, camera([0.; 3]), SIZE).is_empty(),
        "zero-scale hidden bones have no drawable surface"
    );
}

#[test]
fn separate_bones_preserve_empty_space_but_cross_bone_triangles_keep_a_conservative_envelope() {
    let mut rig = rig(&[(7, 3, 0, [0.; 3])], [-0.1; 3], [0.1; 3]);
    let mut vertices = rig
        .geometry_vertices
        .span(rig.geometry_spans[0])
        .unwrap()
        .to_vec();
    let second = vertices
        .iter()
        .copied()
        .map(|mut vertex| {
            vertex.bone_index = 1;
            vertex
        })
        .collect::<Vec<_>>();
    vertices.extend(second);
    rig.geometry_vertices = ActorRigVertexSegments::from_vertices(vertices);
    Arc::make_mut(&mut rig.geometry_spans)[0].vertex_count *= 2;
    Arc::make_mut(&mut rig.manifest)[0].bone_count = 2;
    let mut translated = MATRIX;
    translated[2][3] = 0.5;
    rig.previous_bones = Arc::from([MATRIX, translated]);
    rig.current_bones = Arc::clone(&rig.previous_bones);
    let mut cache = IntersectionCache::default();
    assert!(
        cache.mask(&rig, camera([0., 0., 0.25]), SIZE).is_empty(),
        "a camera between independent bones does not hide the body"
    );
    let mut vertices = rig
        .geometry_vertices
        .span(rig.geometry_spans[0])
        .unwrap()
        .to_vec();
    vertices[1].bone_index = 1;
    rig.geometry_vertices = ActorRigVertexSegments::from_vertices(vertices);
    assert_eq!(
        cache.mask(&rig, camera([0., 0., 0.25]), SIZE).len(),
        1,
        "a polygon spanning different animated bones cannot escape the conservative guard"
    );
}

#[test]
fn rotated_bones_sheared_and_mirrored_world_placement_use_captured_affines() {
    let mut rig = rig(&[(7, 3, 0, [0.; 3])], [-0.2, -0.05, -0.1], [0.2, 0.05, 0.1]);
    let bone = [[0., -1., 0., 1.], [1., 0., 0., 2.], [0., 0., 1., 3.]];
    rig.current_bones = Arc::from([bone]);
    rig.previous_bones = Arc::clone(&rig.current_bones);
    Arc::make_mut(&mut rig.instances)[0].world_from_actor =
        [[-2., 0.5, 0., 0.], [0., 0.25, 0.5, -2.], [0., 0., 3., 0.]];
    let mut cache = IntersectionCache::default();
    assert_eq!(cache.mask(&rig, camera([-1., 0., 9.]), SIZE).len(), 1);
    assert!(cache.mask(&rig, camera([0.; 3]), SIZE).is_empty());
}

#[test]
fn projection_near_plane_is_shared_and_invalid_cameras_cannot_create_masks() {
    let pose = camera([0.; 3]);
    let projection = crate::replay::scene::camera_projection(pose, SIZE);
    assert_eq!(projection.near, render_api::CAMERA_NEAR_PLANE_BLOCKS);
    assert_eq!(projection.near_clip_plane.w, -projection.near);
    let rig = rig(&[(7, 3, 0, [0.; 3])], [-0.25; 3], [0.25; 3]);
    let mut cache = IntersectionCache::default();
    assert!(
        cache
            .mask(
                &rig,
                CameraPose {
                    position: [f64::NAN; 3],
                    ..pose
                },
                SIZE
            )
            .is_empty()
    );
    assert!(cache.mask(&rig, pose, [0, 600]).is_empty());
}

#[test]
fn recorded_third_person_and_spectated_lifetime_policy_remain_independent() {
    let target = target();
    let reference = target.reference();
    let targets = TargetFrame {
        at: MediaTime::ZERO,
        targets: vec![target].into(),
    };
    let body = identity(7, 3, 0);
    let equipment = identity(7, 3, 1);
    let recorded = ActorVisibility::new(7, Some(true), Some(&targets), true, None);
    assert!(recorded.visible(&body));
    assert!(!ActorVisibility::new(7, Some(false), Some(&targets), true, None).visible(&body));
    let spectated = ActorVisibility::new(99, None, Some(&targets), false, Some(reference));
    assert!(!spectated.visible(&body) && !spectated.visible(&equipment));
    assert!(spectated.visible(&identity(7, 4, 0)) && spectated.visible(&identity(8, 3, 0)));
    let stale = CameraTargetRef {
        runtime_id: 7,
        lifetime: Some(TargetLifetime {
            unique_id: 11,
            spawn_revision: 3,
        }),
    };
    assert!(ActorVisibility::new(99, None, Some(&targets), false, Some(stale)).visible(&body));
    assert!(ActorVisibility::new(99, None, None, false, Some(reference)).visible(&body));
}

#[test]
fn camera_only_queries_reuse_geometry_pose_and_gpu_generation_and_prune_other_actors() {
    let draws = (0..render_model::MAX_RENDERED_PLAYERS)
        .map(|index| (index as u64 + 1, 3, 0, [index as f32 * 10., 0., 0.]))
        .collect::<Vec<_>>();
    let source = frozen(rig(&draws, [-0.25; 3], [0.25; 3]));
    let mut cache = IntersectionCache::default();
    let mut publication = crate::replay::actor_frame::Publication::default();
    let mut generation = 0;
    let visibility =
        |mask| ActorVisibility::new(1, Some(false), None, false, None).with_intersections(mask);
    publication
        .prepare(
            &source,
            visibility(cache.mask(&source.sample.rig, camera([0.; 3]), SIZE)),
            false,
            &mut generation,
        )
        .unwrap();
    let vertices = cache.geometry_vertices;
    let builds = cache.pose_builds;
    cache.tested_parts.set(0);
    for yaw in [0., 45., 90., 180., 270.] {
        let pose = CameraPose {
            yaw,
            fov: 100.,
            ..camera([0.; 3])
        };
        assert!(
            publication
                .prepare(
                    &source,
                    visibility(cache.mask(&source.sample.rig, pose, SIZE)),
                    false,
                    &mut generation
                )
                .unwrap()
                .is_none()
        );
    }
    assert_eq!(generation, 1);
    assert_eq!(cache.geometry_vertices, vertices);
    assert_eq!(cache.pose_builds, builds);
    assert_eq!(
        cache.tested_parts.get(),
        5,
        "only the actually overlapping actor reaches per-bone SAT"
    );
    let (_, allocations) = test_alloc::measure(|| {
        for _ in 0..32 {
            assert!(
                cache
                    .mask(&source.sample.rig, camera([-10.; 3]), SIZE)
                    .is_empty()
            );
        }
    });
    assert_eq!(
        allocations.calls, 0,
        "warm empty-space camera edits allocate no collision work"
    );
}

#[test]
fn new_geometry_and_take_replacement_release_retained_source_allocations() {
    let mut source = rig(&[(7, 3, 0, [0.; 3])], [-0.25; 3], [0.25; 3]);
    let weak = Arc::downgrade(&source.geometry_vertices.segments);
    let mut cache = IntersectionCache::default();
    assert_eq!(cache.mask(&source, camera([0.; 3]), SIZE).len(), 1);
    let count = cache.geometry_vertices;
    source = rig(&[(7, 4, 0, [3., 0., 0.])], [-0.1; 3], [0.1; 3]);
    assert!(cache.mask(&source, camera([0.; 3]), SIZE).is_empty());
    assert!(weak.upgrade().is_none());
    assert!(cache.geometry_vertices > count);
    let weak = Arc::downgrade(&source.current_bones);
    drop(source);
    assert!(weak.upgrade().is_some());
    cache.clear();
    assert!(weak.upgrade().is_none());
}
