//! GPU removal carriers under publication: acknowledgement reservation failures, same-key
//! supersession and the urgent reserve and its ordering.

use super::*;

#[test]
fn failed_gpu_removal_ack_reservation_requeues_without_retiring_or_leaking_an_ack() {
    use bevy::ecs::system::RunSystemOnce;

    let key = SubChunkKey::new(0, 8, 0, 9);
    let token = ChunkUploadToken {
        generation: 42,
        dirty_since: Instant::now(),
    };
    let allowance =
        render_api::PublicationAllowance::new(render_api::PublicationServiceConfig::DEFAULT);
    allowance.begin_frame(
        1,
        1,
        0,
        1,
        render_api::PublicationServiceConfig::DEFAULT.maximum_frame_items,
    );
    let permit = allowance
        .try_admit_zero_byte()
        .unwrap()
        .into_handoff()
        .unwrap()
        .into_render_entity()
        .unwrap();
    let acknowledgements = ChunkUploadAcknowledgements::with_capacity(0);
    let gpu_removals = ChunkGpuRemovalQueue::default();
    assert!(
        gpu_removals
            .push(PendingGpuRemoval {
                publications: None,
                kind: render_api::TerrainRemovalKind::Empty,
                key,
                token: Some(token),
                priority: ChunkUploadPriority::new(0.0),
                permit,
            })
            .is_ok()
    );
    let mut app = noop_gpu_publication_app(acknowledgements.clone(), gpu_removals.clone());

    app.world_mut().run_system_once(prepare_gpu_chunks).unwrap();

    assert!(acknowledgements.is_empty());
    assert!(acknowledgements.drain().is_empty());
    assert_eq!(gpu_removals.pending_len(), 1);
    assert_eq!(allowance.live_permits(), 1);
    drop(app);
    drop(gpu_removals);
    assert_eq!(allowance.live_permits(), 0);
}

#[test]
fn a_newer_gpu_removal_supersedes_and_retires_the_same_key_carrier() {
    let key = SubChunkKey::new(0, 9, 0, 9);
    let allowance =
        render_api::PublicationAllowance::new(render_api::PublicationServiceConfig::DEFAULT);
    allowance.begin_frame(
        1,
        2,
        0,
        2,
        render_api::PublicationServiceConfig::DEFAULT.maximum_frame_items,
    );
    let first = allowance
        .try_admit_zero_byte()
        .unwrap()
        .into_handoff()
        .unwrap()
        .into_render_entity()
        .unwrap();
    let second = allowance
        .try_admit_zero_byte()
        .unwrap()
        .into_handoff()
        .unwrap()
        .into_render_entity()
        .unwrap();
    let gpu_removals = ChunkGpuRemovalQueue::default();
    assert!(
        gpu_removals
            .push(PendingGpuRemoval {
                publications: None,
                kind: render_api::TerrainRemovalKind::Empty,
                key,
                token: None,
                priority: ChunkUploadPriority::new(0.0),
                permit: first,
            })
            .is_ok()
    );
    assert_eq!(allowance.live_permits(), 2);

    assert!(
        gpu_removals
            .push(PendingGpuRemoval {
                publications: None,
                kind: render_api::TerrainRemovalKind::Empty,
                key,
                token: None,
                priority: ChunkUploadPriority::new(0.0),
                permit: second,
            })
            .is_ok()
    );
    assert_eq!(gpu_removals.pending_len(), 1);
    assert_eq!(allowance.live_permits(), 1);

    gpu_removals.cancel(key);
    assert_eq!(gpu_removals.pending_len(), 0);
    assert_eq!(allowance.live_permits(), 0);
}

#[test]
fn ordinary_gpu_removals_cannot_consume_the_urgent_reserve() {
    let config = render_api::PublicationServiceConfig::DEFAULT;
    let allowance = render_api::PublicationAllowance::new(config);
    let gpu_removals = ChunkGpuRemovalQueue::default();
    allowance.begin_frame(
        1,
        config.maximum_zero_byte_operations_per_frame,
        0,
        config.maximum_zero_byte_operations_per_frame,
        config.maximum_frame_items,
    );
    for index in 0..config.maximum_zero_byte_operations_per_frame {
        let permit = allowance
            .try_admit_zero_byte()
            .unwrap()
            .into_handoff()
            .unwrap()
            .into_render_entity()
            .unwrap();
        assert!(
            gpu_removals
                .push(PendingGpuRemoval {
                    publications: None,
                    kind: render_api::TerrainRemovalKind::Empty,
                    key: SubChunkKey::new(0, index as i32, 0, 0),
                    token: None,
                    priority: ChunkUploadPriority::new(0.0),
                    permit,
                })
                .is_ok()
        );
    }

    allowance.begin_frame(2, 2, 0, 2, config.maximum_frame_items);
    let ordinary_overflow = allowance
        .try_admit_zero_byte_with_priority(true)
        .unwrap()
        .into_handoff()
        .unwrap()
        .into_render_entity()
        .unwrap();
    assert!(
        gpu_removals
            .push(PendingGpuRemoval {
                publications: None,
                kind: render_api::TerrainRemovalKind::Empty,
                key: SubChunkKey::new(0, 256, 0, 0),
                token: None,
                priority: ChunkUploadPriority::new(0.0),
                permit: ordinary_overflow,
            })
            .is_err()
    );
    let urgent = allowance
        .try_admit_zero_byte_with_priority(true)
        .unwrap()
        .into_handoff()
        .unwrap()
        .into_render_entity()
        .unwrap();
    assert!(
        gpu_removals
            .push(PendingGpuRemoval {
                publications: None,
                kind: render_api::TerrainRemovalKind::Empty,
                key: SubChunkKey::new(0, 257, 0, 0),
                token: None,
                priority: ChunkUploadPriority::urgent(),
                permit: urgent,
            })
            .is_ok()
    );
    assert_eq!(
        gpu_removals.pending_len(),
        config.maximum_zero_byte_operations_per_frame + 1
    );

    drop(gpu_removals);
    assert_eq!(allowance.live_permits(), 0);
}

#[test]
fn older_urgent_gpu_removals_stay_ahead_of_new_urgent_work() {
    let config = render_api::PublicationServiceConfig::DEFAULT;
    let allowance = render_api::PublicationAllowance::new(config);
    let gpu_removals = ChunkGpuRemovalQueue::default();
    allowance.begin_frame(
        1,
        config.urgent_zero_byte_reserve,
        0,
        config.urgent_zero_byte_reserve,
        config.maximum_frame_items,
    );
    for index in 0..config.urgent_zero_byte_reserve {
        let permit = allowance
            .try_admit_zero_byte_with_priority(true)
            .unwrap()
            .into_handoff()
            .unwrap()
            .into_render_entity()
            .unwrap();
        assert!(
            gpu_removals
                .push(PendingGpuRemoval {
                    publications: None,
                    kind: render_api::TerrainRemovalKind::Empty,
                    key: SubChunkKey::new(0, index as i32, 0, 0),
                    token: None,
                    priority: ChunkUploadPriority::urgent(),
                    permit,
                })
                .is_ok()
        );
    }

    allowance.begin_frame(
        2,
        config.maximum_zero_byte_operations_per_frame,
        0,
        config.maximum_zero_byte_operations_per_frame,
        config.maximum_frame_items,
    );
    for index in config.urgent_zero_byte_reserve
        ..config.maximum_zero_byte_operations_per_frame + config.urgent_zero_byte_reserve
    {
        let permit = allowance
            .try_admit_zero_byte_with_priority(true)
            .unwrap()
            .into_handoff()
            .unwrap()
            .into_render_entity()
            .unwrap();
        assert!(
            gpu_removals
                .push(PendingGpuRemoval {
                    publications: None,
                    kind: render_api::TerrainRemovalKind::Empty,
                    key: SubChunkKey::new(0, index as i32, 0, 0),
                    token: None,
                    priority: ChunkUploadPriority::urgent(),
                    permit,
                })
                .is_ok()
        );
    }

    let completed =
        gpu_removals.take_ready(config.maximum_zero_byte_operations_per_frame, |_| true);
    assert_eq!(
        completed
            .iter()
            .take(config.urgent_zero_byte_reserve)
            .map(|removal| removal.key.x)
            .collect::<Vec<_>>(),
        (0..config.urgent_zero_byte_reserve as i32).collect::<Vec<_>>()
    );
    assert_eq!(gpu_removals.pending_len(), config.urgent_zero_byte_reserve);

    drop(completed);
    drop(gpu_removals);
    assert_eq!(allowance.live_permits(), 0);
}
