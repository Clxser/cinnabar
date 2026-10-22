use super::*;

/// Shared accounting includes the actual weak owner, queue charge and indexed identity metadata.
fn shared_cost<T: ?Sized>(bytes: usize) -> usize {
    bytes + CHARGE_METADATA + INDEX_METADATA + std::mem::size_of::<Weak<T>>()
}

#[test]
fn queued_frames_charge_unchanged_pixels_once() {
    let mut budget = Budget::default();
    let mut frame = FrozenVisuals::default();
    frame.particles.base = Some(vec![0u8; 24 * 1024 * 1024].into());
    frame.blocks.dynamic_rgba8 = Arc::clone(frame.particles.base.as_ref().unwrap());
    let mut leases = Vec::new();
    let mut owned = 0;
    for _ in 0..24 {
        let (lease, bytes) = budget.retain(&frame).unwrap();
        owned += bytes;
        leases.push(lease);
    }
    assert!(24 * frame.retained_bytes() > MAX_QUEUED_BYTES);
    assert_eq!(
        budget.used.load(Ordering::Acquire),
        shared_cost::<[u8]>(24 * 1024 * 1024) + owned
    );
    drop(leases);
    assert_eq!(budget.used.load(Ordering::Acquire), 0);
}

#[test]
fn shared_cells_survive_an_atlas_wrapper_change_without_double_charging_pixels() {
    use render_model::{NametagAtlasRect, NametagScene};
    let mut frame = FrozenVisuals::default();
    let pixels: Arc<[u8]> = vec![0u8; 1024].into();
    frame.nametags = Some(Arc::new(NametagScene {
        atlas: vec![NametagAtlasRect {
            cell: [0, 0, 16, 16],
            rgba8: Arc::clone(&pixels),
        }]
        .into(),
        ..Default::default()
    }));
    let mut budget = Budget::default();
    let (first, first_owned) = budget.retain(&frame).unwrap();
    let mut next = (**frame.nametags.as_ref().unwrap()).clone();
    next.atlas = next.atlas.to_vec().into();
    frame.nametags = Some(Arc::new(next));
    let (second, second_owned) = budget.retain(&frame).unwrap();
    assert_eq!(
        budget.used.load(Ordering::Acquire),
        shared_cost::<[u8]>(pixels.len())
            + 2 * shared_cost::<NametagScene>(std::mem::size_of::<NametagScene>())
            + 2 * shared_cost::<[NametagAtlasRect]>(std::mem::size_of::<NametagAtlasRect>())
            + 2 * epochs::group_metadata(2)
            + first_owned
            + second_owned
    );
    drop((first, second));
    assert_eq!(budget.used.load(Ordering::Acquire), 0);
}

#[test]
fn distinct_allocations_still_exhaust_the_original_budget_and_failed_reservations_release_bytes() {
    let mut budget = Budget::default();
    let a: Arc<[u8]> = vec![0u8; 1].into();
    let b: Arc<[u8]> = vec![1u8; 1].into();
    let mut first = Lease::default();
    let metadata = shared_cost::<[u8]>(0);
    budget
        .shared(&a, MAX_QUEUED_BYTES - metadata, &mut first)
        .unwrap();
    let mut second = Lease::default();
    assert!(budget.shared(&b, 1, &mut second).is_err());
    assert_eq!(budget.used.load(Ordering::Acquire), MAX_QUEUED_BYTES);
    drop(first);
    budget.shared(&b, 1, &mut second).unwrap();
    assert_eq!(budget.used.load(Ordering::Acquire), shared_cost::<[u8]>(1));
    drop(second);
    assert_eq!(budget.used.load(Ordering::Acquire), 0);
}

#[test]
fn rejected_frame_releases_new_shared_charges_without_releasing_other_queued_samples() {
    let mut budget = Budget::default();
    let old: Arc<[u8]> = vec![0u8; 1].into();
    let mut retained = Lease::default();
    budget
        .shared(
            &old,
            MAX_QUEUED_BYTES - 4096 - shared_cost::<[u8]>(0),
            &mut retained,
        )
        .unwrap();
    let before = budget.used.load(Ordering::Acquire);
    let mut frame = FrozenVisuals::default();
    frame.particles.base = Some(vec![0u8; 4096].into());
    assert!(budget.retain(&frame).is_err());
    assert_eq!(budget.used.load(Ordering::Acquire), before);
    drop(retained);
    assert_eq!(budget.used.load(Ordering::Acquire), 0);
}

#[test]
fn republished_item_catalogs_share_their_child_pixels_across_queued_epochs() {
    use render_model::{DroppedItemModel, DroppedItemSprite};
    let pixels: Arc<[u8]> = vec![0; 64 * 1024 * 1024].into();
    let model = DroppedItemModel::NativeSprite(DroppedItemSprite {
        width: 4096,
        height: 4096,
        rgba8: Arc::clone(&pixels),
    });
    let mut budget = Budget::default();
    let mut leases = Vec::new();
    let mut owned = 0;
    for _ in 0..7 {
        let mut frame = FrozenVisuals::default();
        frame.items = Some(super::super::items::PreparedItems {
            models: vec![model.clone()].into(),
            ..Default::default()
        });
        let (lease, bytes) = budget.retain(&frame).unwrap();
        owned += bytes;
        leases.push(lease);
    }
    assert_eq!(
        budget.used.load(Ordering::Acquire),
        shared_cost::<[u8]>(pixels.len())
            + 7 * shared_cost::<[DroppedItemModel]>(std::mem::size_of::<DroppedItemModel>())
            + 7 * epochs::group_metadata(2)
            + owned
    );
    drop(leases);
    assert_eq!(budget.used.load(Ordering::Acquire), 0);
}

/// Reusable sprites keep source data synthetic and permit precise allocation sharing between epochs.
fn sprite(pixels: &Arc<[u8]>) -> render_model::DroppedItemModel {
    render_model::DroppedItemModel::NativeSprite(render_model::DroppedItemSprite {
        width: 1,
        height: 1,
        rgba8: Arc::clone(pixels),
    })
}

#[test]
fn unchanged_nested_catalogs_attach_one_group_without_rewalking_children_or_repinning_allocations()
{
    use render_model::{NametagAtlasRect, NametagScene};
    let pixels: Arc<[u8]> = vec![1; 4].into();
    let mut frame = FrozenVisuals::default();
    frame.items = Some(super::super::items::PreparedItems {
        models: (0..128).map(|_| sprite(&pixels)).collect::<Vec<_>>().into(),
        ..Default::default()
    });
    frame.nametags = Some(Arc::new(NametagScene {
        atlas: (0..64)
            .map(|index| NametagAtlasRect {
                cell: [index, 0, 1, 1],
                rgba8: Arc::clone(&pixels),
            })
            .collect::<Vec<_>>()
            .into(),
        ..Default::default()
    }));
    let mut budget = Budget::default();
    let (first, _) = budget.retain(&frame).unwrap();
    let (models, cells, groups, pins) = (
        budget.work.models,
        budget.work.cells,
        budget.work.groups,
        budget.work.pins,
    );
    assert_eq!((models, cells, groups), (128, 64, 2));
    let mut retained = vec![first];
    for _ in 0..60 {
        let (lease, _) = budget.retain(&frame).unwrap();
        assert_eq!(lease.groups.len(), 2);
        assert!(Arc::ptr_eq(&lease.groups[0], &retained[0].groups[0]));
        assert!(Arc::ptr_eq(&lease.groups[1], &retained[0].groups[1]));
        retained.push(lease);
    }
    assert_eq!(
        (
            budget.work.models,
            budget.work.cells,
            budget.work.groups,
            budget.work.pins
        ),
        (models, cells, groups, pins)
    );
    assert_eq!(
        budget.work.logical,
        frame.retained_bytes() + SAMPLE_OVERHEAD
    );
    assert!(budget.models.metadata_bytes() <= epochs::MAX_DESCRIPTOR_BYTES);
    assert!(budget.atlas.metadata_bytes() <= epochs::MAX_DESCRIPTOR_BYTES);
    drop(retained);
    assert_eq!(budget.used.load(Ordering::Acquire), 0);
    assert_eq!(budget.live_keys.load(Ordering::Acquire), 0);
    // Descriptor projections do not reserve child pixel allocations after the last queue owner retires.
    assert_eq!(Arc::weak_count(&pixels), 0);
}

#[test]
fn cold_reactivation_reuses_layouts_and_weak_root_identity_rejects_mutated_catalogs() {
    let pixels: Arc<[u8]> = vec![1; 4].into();
    let mut frame = FrozenVisuals::default();
    frame.items = Some(super::super::items::PreparedItems {
        models: vec![sprite(&pixels)].into(),
        ..Default::default()
    });
    let mut budget = Budget::default();
    let (first, _) = budget.retain(&frame).unwrap();
    drop(first);
    assert_eq!(budget.used.load(Ordering::Acquire), 0);
    let first_visits = budget.work.models;
    let (second, _) = budget.retain(&frame).unwrap();
    assert_eq!(budget.work.models, first_visits);
    assert_eq!(budget.work.groups, 2);
    drop(second);
    let changed: Arc<[u8]> = vec![2; 12].into();
    // Even unique roots with a cached weak descriptor must dissociate before mutation is possible.
    let models = &mut frame.items.as_mut().unwrap().models;
    assert!(Arc::get_mut(models).is_none());
    Arc::make_mut(models)[0] = sprite(&changed);
    let (third, owned) = budget.retain(&frame).unwrap();
    assert_eq!(budget.work.models, first_visits + 1);
    assert_eq!(
        budget.used.load(Ordering::Acquire),
        shared_cost::<[render_model::DroppedItemModel]>(std::mem::size_of::<
            render_model::DroppedItemModel,
        >()) + shared_cost::<[u8]>(changed.len())
            + epochs::group_metadata(2)
            + owned
    );
    assert_eq!(
        budget.work.logical,
        frame.retained_bytes() + SAMPLE_OVERHEAD
    );
    drop(third);
    assert_eq!(budget.used.load(Ordering::Acquire), 0);
}

#[test]
fn republished_wrapper_with_one_new_child_charges_only_new_allocations_and_keeps_shared_children_live()
 {
    let old: Arc<[u8]> = vec![1; 4].into();
    let new: Arc<[u8]> = vec![2; 8].into();
    let mut frame = FrozenVisuals::default();
    frame.items = Some(super::super::items::PreparedItems {
        models: vec![sprite(&old)].into(),
        ..Default::default()
    });
    let mut budget = Budget::default();
    let (first, first_owned) = budget.retain(&frame).unwrap();
    frame.items.as_mut().unwrap().models = vec![sprite(&old), sprite(&new)].into();
    let pins = budget.work.pins;
    let (second, second_owned) = budget.retain(&frame).unwrap();
    assert_eq!(budget.work.pins - pins, 2); // one catalog root and the newly introduced pixel owner
    let model_bytes = std::mem::size_of::<render_model::DroppedItemModel>();
    assert_eq!(
        budget.used.load(Ordering::Acquire),
        shared_cost::<[render_model::DroppedItemModel]>(model_bytes)
            + shared_cost::<[render_model::DroppedItemModel]>(2 * model_bytes)
            + shared_cost::<[u8]>(old.len())
            + shared_cost::<[u8]>(new.len())
            + epochs::group_metadata(2)
            + epochs::group_metadata(3)
            + first_owned
            + second_owned
    );
    drop(first);
    assert_eq!(
        budget.used.load(Ordering::Acquire),
        shared_cost::<[render_model::DroppedItemModel]>(2 * model_bytes)
            + shared_cost::<[u8]>(old.len())
            + shared_cost::<[u8]>(new.len())
            + epochs::group_metadata(3)
            + second_owned
    );
    drop(second);
    assert_eq!(budget.used.load(Ordering::Acquire), 0);
}

#[test]
fn failed_group_or_final_frame_admission_rolls_back_all_new_charges_and_can_retry_cached_descriptors()
 {
    for fail_group in [false, true] {
        let pixels: Arc<[u8]> = vec![1; 16].into();
        let mut frame = FrozenVisuals::default();
        frame.items = Some(super::super::items::PreparedItems {
            models: vec![sprite(&pixels)].into(),
            ..Default::default()
        });
        let mut budget = Budget::default();
        let root = shared_cost::<[render_model::DroppedItemModel]>(std::mem::size_of::<
            render_model::DroppedItemModel,
        >());
        let available = if fail_group {
            root + 8
        } else {
            root + shared_cost::<[u8]>(pixels.len()) + epochs::group_metadata(2)
        };
        let blocker = budget.charge(MAX_QUEUED_BYTES - available, None).unwrap();
        let before = budget.used.load(Ordering::Acquire);
        assert!(budget.retain(&frame).is_err());
        assert_eq!(budget.used.load(Ordering::Acquire), before);
        assert_eq!(budget.live_keys.load(Ordering::Acquire), 0);
        assert_eq!(Arc::weak_count(&pixels), 0);
        assert!(budget.models.metadata_bytes() <= epochs::MAX_DESCRIPTOR_BYTES);
        let visits = budget.work.models;
        drop(blocker);
        let (retry, _) = budget.retain(&frame).unwrap();
        assert_eq!(budget.work.models, visits);
        drop(retry);
        assert_eq!(budget.used.load(Ordering::Acquire), 0);
    }
}

#[test]
fn oversized_descriptor_metadata_falls_back_without_rejecting_an_admissible_payload() {
    let count =
        epochs::MAX_DESCRIPTOR_BYTES / std::mem::size_of::<render_model::DroppedItemModel>() + 1;
    let mut frame = FrozenVisuals::default();
    frame.items = Some(super::super::items::PreparedItems {
        models: vec![render_model::DroppedItemModel::Vacant; count].into(),
        ..Default::default()
    });
    let mut budget = Budget::default();
    let (lease, _) = budget.retain(&frame).unwrap();
    assert_eq!(budget.models.metadata_bytes(), 0);
    assert!(lease.groups.is_empty());
    assert_eq!(
        budget.work.logical,
        frame.retained_bytes() + SAMPLE_OVERHEAD
    );
    drop(lease);
    assert_eq!(budget.used.load(Ordering::Acquire), 0);
}

#[test]
fn archive_admission_rejection_drops_the_whole_epoch_lease_before_any_worker_owns_it() {
    use recording_studio::{
        MediaTime,
        archive::{RECORDING_VERSION, RecordKind, Recording, RecordingInfo},
    };
    let root = tempfile::tempdir().unwrap();
    let mut recording = Recording::start_ready(
        root.path().join("take"),
        RecordingInfo {
            version: RECORDING_VERSION,
            name: "Visual queue fixture".into(),
            game_version: "fixture".into(),
            protocol_version: 0,
            scene_schema: super::super::VISUAL_TRACK_SCHEMA.into(),
            created_unix_ms: 0,
            viewport: [16, 16],
            local_actor: 1,
        },
        Default::default(),
    )
    .unwrap();
    let pixels: Arc<[u8]> = vec![1; 4].into();
    let mut frame = FrozenVisuals::default();
    frame.items = Some(super::super::items::PreparedItems {
        models: vec![sprite(&pixels)].into(),
        ..Default::default()
    });
    let mut budget = Budget::default();
    let (lease, bytes) = budget.retain(&frame).unwrap();
    assert!(budget.used.load(Ordering::Acquire) > pixels.len());
    let task = recording.task();
    assert!(
        recording
            .push_prepared(
                MediaTime(recording_studio::time::MAX_MEDIA_MICROS + 1),
                RecordKind::Scene,
                bytes,
                move |_| {
                    drop(lease);
                    panic!("Rejected visual payload must never execute")
                }
            )
            .is_err()
    );
    assert_eq!(budget.used.load(Ordering::Acquire), 0);
    assert_eq!(budget.live_keys.load(Ordering::Acquire), 0);
    assert_eq!(Arc::weak_count(&pixels), 0);
    let _ = recording.finish(MediaTime::ZERO);
    task.wait();
}

#[test]
fn incremental_and_threshold_reclamation_drop_dead_keys_without_touching_live_queue_charges() {
    let mut budget = Budget::default();
    let owners = (0..40)
        .map(|value| Arc::<[u8]>::from(vec![value]))
        .collect::<Vec<_>>();
    let mut live = Lease::default();
    for owner in &owners[..2] {
        budget.allocation(owner, &mut live).unwrap();
    }
    for owner in &owners[2..] {
        let mut retired = Lease::default();
        budget.allocation(owner, &mut retired).unwrap();
    }
    let before = budget.used.load(Ordering::Acquire);
    let steps = budget.work.pruned;
    budget.prune();
    assert_eq!(budget.work.pruned - steps, PRUNE_STEPS);
    assert_eq!(budget.used.load(Ordering::Acquire), before);
    budget.bound_index(1);
    assert_eq!(budget.allocations.len(), 2);
    assert_eq!(budget.live_keys.load(Ordering::Acquire), 2);
    assert_eq!(budget.used.load(Ordering::Acquire), before);
    drop(live);
    budget.prune();
    budget.prune();
    assert!(budget.allocations.is_empty());
    assert_eq!(budget.used.load(Ordering::Acquire), 0);
}

#[test]
fn diverse_nested_models_and_dynamic_allocations_match_the_source_owned_logical_total() {
    use render_model::{
        DroppedItemBlock, DroppedItemCube, DroppedItemModel, DroppedItemSprite, NametagAtlasRect,
        NametagScene,
    };
    let pixels: Arc<[u8]> = vec![3; 16].into();
    let sprite = DroppedItemSprite {
        width: 2,
        height: 2,
        rgba8: Arc::clone(&pixels),
    };
    let mut frame = FrozenVisuals::default();
    frame.items = Some(super::super::items::PreparedItems {
        models: vec![
            DroppedItemModel::Vacant,
            DroppedItemModel::Sprite(sprite.clone()),
            DroppedItemModel::NativeSprite(sprite.clone()),
            DroppedItemModel::Cube(DroppedItemCube {
                tile: 2,
                faces: std::array::from_fn(|_| Arc::clone(&pixels)),
                tints: [u32::MAX; 6],
            }),
            DroppedItemModel::Block(DroppedItemBlock {
                materials: vec![(sprite, u32::MAX)].into(),
                quads: Arc::from([]),
                rotation: 0,
            }),
        ]
        .into(),
        ..Default::default()
    });
    frame.nametags = Some(Arc::new(NametagScene {
        records: vec![Default::default()],
        atlas: vec![NametagAtlasRect {
            cell: [0, 0, 2, 2],
            rgba8: Arc::clone(&pixels),
        }]
        .into(),
        ..Default::default()
    }));
    frame.particles.base = Some(Arc::clone(&pixels));
    frame.particles.patches = vec![particles::AtlasPatch {
        seq: 1,
        x: 0,
        y: 0,
        width: 2,
        height: 2,
        rgba8: Arc::clone(&pixels),
    }]
    .into();
    frame.blocks.dynamic_rgba8 = Arc::clone(&pixels);
    frame.atmosphere = Some(super::super::atmosphere::AtmosphereVisuals {
        assets: Arc::new(super::super::atmosphere::AtmosphereAssets {
            pumpkin: Some(Arc::clone(&pixels)),
            ..Default::default()
        }),
        precipitation: Arc::from([]),
        forward_offset: [0.0; 3],
        occlusion_origin: [0; 2],
        occlusion_heights: vec![1, 2, 3].into(),
        occlusion_generation: 1,
        lightning: vec![[0.0; 8]].into(),
        overlays: Arc::from([]),
        overlay_clock: 0.0,
        fire_projection: [0.0; 2],
        portal_from_clip: [[0.0; 4]; 4],
        fog_weather_level: 0.0,
        current_rain_level: 0.0,
        clouds: true,
        skybox_visible: true,
        skybox_native_offset: false,
    });
    let mut budget = Budget::default();
    let (lease, _) = budget.retain(&frame).unwrap();
    assert_eq!(
        budget.work.logical,
        frame.retained_bytes() + SAMPLE_OVERHEAD
    );
    let (second, _) = budget.retain(&frame).unwrap();
    assert_eq!(
        budget.work.logical,
        frame.retained_bytes() + SAMPLE_OVERHEAD
    );
    drop((lease, second));
    assert_eq!(budget.used.load(Ordering::Acquire), 0);
    budget.retain(&FrozenVisuals::default()).unwrap();
    assert_eq!(budget.models.metadata_bytes(), 0);
    assert_eq!(budget.atlas.metadata_bytes(), 0);
}
