use super::*;

/// Installs a solved emitter section whose boundary light uses packed storage.
fn packed_stream() -> (WorldStream, SubChunkKey) {
    let mut stream = lit_stream(1);
    let key = SubChunkKey::new(1, 0, 0, 0);
    stream
        .authority
        .update_block(key, BlockUpdate::new(8, 8, 8, 0, 1), 0)
        .unwrap();
    stream.resident.insert(key);
    stream.mark_changed(key, Instant::now());
    complete_one_light(&mut stream, [8.0; 3]);
    (stream, key)
}

/// Solves an unchanged packed section against the same immutable prior held by acceptance.
fn packed_completion() -> (WorldStream, LightCompletion) {
    let (mut stream, key) = packed_stream();
    stream.mark_light_dirty_exact(key).unwrap();
    let job = stream.take_pending_light_job(key);
    let completion = LightCompletion {
        key,
        identity: job.identity,
        result: solve_prepared_light_job(job),
        queue_wait: Duration::ZERO,
        duration: Duration::ZERO,
    };
    assert!(
        !completion
            .result
            .as_ref()
            .unwrap()
            .replacement
            .channel(LightChannel::Block)
            .is_uniform()
    );
    (stream, completion)
}

/// Prepares a chosen solver output through the same result owner as real workers.
fn prepare_replacement(
    stream: &mut WorldStream,
    key: SubChunkKey,
    replacement: SubChunkLight,
    direct: DirectSkyMask,
) -> LightCompletion {
    stream.mark_light_dirty_exact(key).unwrap();
    let job = stream.take_pending_light_job(key);
    LightCompletion {
        key,
        identity: job.identity,
        result: Ok(lighting::results::LightBatchTarget::capture(&job).finish(
            replacement.with_generation(job.identity.revision),
            Arc::new(direct),
            false,
        )),
        queue_wait: Duration::ZERO,
        duration: Duration::ZERO,
    }
}

/// Makes one direct bit without changing the other cells on any face.
fn direct_bit([x, y, z]: [u8; 3]) -> DirectSkyMask {
    let mut words = Box::new([0_u64; 64]);
    let index = light_local_index(x, y, z);
    words[index / 64] |= 1 << (index % 64);
    DirectSkyMask::Packed(words)
}

/// Compares fused outputs and sampled channel pairs with the unchanged separate predicates.
fn assert_face_comparison(
    stream: &WorldStream,
    key: SubChunkKey,
    replacement: &SubChunkLight,
    direct: &DirectSkyMask,
) -> ([usize; 2], [usize; 2]) {
    let previous = stream.lighting.store.light(key).cloned();
    let raw_direct = stream.lighting.direct_sky.get(&key).cloned();
    let valid_direct = raw_direct.as_ref().filter(|old| {
        previous
            .as_ref()
            .is_some_and(|light| old.light_revision == light.generation())
    });
    let (expected, separate_work) = lighting::work::measure(|| {
        (
            LIGHT_NEIGHBOUR_OFFSETS.map(|offset| {
                light_face_changed(
                    previous.as_deref(),
                    replacement,
                    valid_direct.map(|old| old.mask.as_ref()),
                    direct,
                    offset,
                )
            }),
            stream.monotonic_light_faces(key, replacement, direct),
        )
    });
    let ((changed, prepared), fused_work) = lighting::work::measure(|| {
        lighting::results::PreparedMonotonicFaces::prepare(
            previous,
            raw_direct,
            replacement,
            direct,
        )
    });
    let monotonic = prepared
        .matching(
            stream.lighting.store.light(key),
            stream.lighting.direct_sky.get(&key),
        )
        .unwrap();
    assert_eq!((changed, monotonic), expected);
    assert!(
        fused_work[0] <= separate_work[0],
        "face visits: fused={fused_work:?}, separate={separate_work:?}"
    );
    assert!(
        fused_work[1] <= separate_work[1],
        "channel pairs: fused={fused_work:?}, separate={separate_work:?}"
    );
    (separate_work, fused_work)
}

#[test]
fn prepared_packed_completion_accepts_without_owner_face_visits() {
    let (mut stream, completion) = packed_completion();
    let accepted = stream.stats().accepted_light_jobs;
    let noops = stream.stats().noop_light_jobs;

    let (_, work) = lighting::work::measure(|| stream.accept_light_completion(completion));

    assert_eq!(stream.stats().accepted_light_jobs, accepted + 1);
    assert_eq!(stream.stats().noop_light_jobs, noops + 1);
    assert_eq!(
        work,
        [0, 0],
        "prepared faces must not be compared again on acceptance"
    );
}

#[test]
fn unchanged_packed_worker_reuses_exact_complete_equality() {
    let (mut stream, key) = packed_stream();
    stream.mark_light_dirty_exact(key).unwrap();
    let job = stream.take_pending_light_job(key);

    let (result, work) = lighting::work::measure(|| solve_prepared_light_job(job).unwrap());

    assert!(!result.light_levels_changed);
    assert!(!result.direct_sky_changed);
    assert_eq!(result.changed_faces, [false; 6]);
    assert_eq!(
        work,
        [0, 0],
        "equal complete outputs need no worker boundary scan"
    );
    assert_eq!(
        result.monotonic_faces.matching(
            stream.lighting.store.light(key),
            stream.lighting.direct_sky.get(&key)
        ),
        Some([true; 6])
    );
}

#[test]
fn fused_faces_match_each_packed_channel_direction_and_provenance_change() {
    let mut stream = lit_stream(1);
    let key = SubChunkKey::new(1, 0, 0, 0);
    for offset in LIGHT_NEIGHBOUR_OFFSETS {
        let position = sampled_light_shell(offset)
            .nth(world::SUB_CHUNK_SIDE * 5 + 7)
            .unwrap();
        for channel in [LightChannel::Block, LightChannel::Sky] {
            for level in [6, 7, 8] {
                install_current_light(&mut stream, key, 7, 7, false);
                let generation = stream.lighting.store.light(key).unwrap().generation();
                let mut previous = SubChunkLight::uniform(7, 7, generation).unwrap();
                previous.set(LightChannel::Block, 8, 8, 8, 8).unwrap();
                let mut replacement = previous.clone().with_generation(generation + 1);
                replacement
                    .set(channel, position[0], position[1], position[2], level)
                    .unwrap();
                stream.lighting.store.insert_known_air(key, previous);
                let (separate, fused) = assert_face_comparison(
                    &stream,
                    key,
                    &replacement,
                    &DirectSkyMask::Uniform(false),
                );
                if level == 7 {
                    let cells = LIGHT_NEIGHBOUR_OFFSETS.len() * world::SUB_CHUNK_SIDE.pow(2);
                    assert_eq!(separate, [cells * 2, cells * 4]);
                    assert_eq!(fused, [cells, cells * 2]);
                }
            }
        }
        for gained in [false, true] {
            let (mut stream, key) = packed_stream();
            let replacement = stream.lighting.store.light(key).unwrap().as_ref().clone();
            if gained {
                assert_face_comparison(&stream, key, &replacement, &direct_bit(position));
            } else {
                stream.lighting.direct_sky.get_mut(&key).unwrap().mask =
                    Arc::new(direct_bit(position));
                assert_face_comparison(&stream, key, &replacement, &DirectSkyMask::Uniform(false));
            }
        }
    }
}

#[test]
fn fused_faces_keep_missing_and_invalid_prior_provenance_distinct() {
    let mut stream = lit_stream(1);
    let key = SubChunkKey::new(1, 0, 0, 0);
    for shape in 0..4 {
        for provenance in 0..3 {
            install_current_light(&mut stream, key, 0, if shape == 2 { 7 } else { 0 }, false);
            if shape == 0 {
                stream.lighting.store.remove(key);
            } else if shape == 3 {
                let mut previous = stream.lighting.store.light(key).unwrap().as_ref().clone();
                previous.set(LightChannel::Sky, 8, 8, 8, 1).unwrap();
                stream.lighting.store.insert_known_air(key, previous);
            }
            match provenance {
                0 => {
                    stream.lighting.direct_sky.remove(&key);
                }
                1 => {}
                _ => {
                    stream
                        .lighting
                        .direct_sky
                        .get_mut(&key)
                        .unwrap()
                        .light_revision += 1;
                }
            }
            assert_face_comparison(
                &stream,
                key,
                &SubChunkLight::dark(50_000),
                &DirectSkyMask::Uniform(false),
            );
        }
    }
}

#[test]
fn worker_capture_preserves_invalid_zero_sky_provenance_through_acceptance() {
    for invalid in [false, true] {
        let mut stream = lit_stream(1);
        let key = SubChunkKey::new(1, 0, 0, 0);
        let neighbour = SubChunkKey::new(1, 1, 0, 0);
        install_current_light(&mut stream, key, 0, 0, false);
        install_current_light(&mut stream, neighbour, 0, 0, false);
        if invalid {
            stream
                .lighting
                .direct_sky
                .get_mut(&key)
                .unwrap()
                .light_revision += 1;
        } else {
            stream.lighting.direct_sky.remove(&key);
        }
        let completion = prepare_replacement(
            &mut stream,
            key,
            SubChunkLight::dark(0),
            DirectSkyMask::Uniform(false),
        );
        stream
            .lighting
            .waiters
            .entry(key)
            .or_default()
            .insert(neighbour);
        let before = stream.stats();

        let (_, work) = lighting::work::measure(|| stream.accept_light_completion(completion));

        assert_eq!(
            work,
            [0, 0],
            "matching worker inputs need no owner comparison"
        );
        assert_eq!(
            stream.lighting.jobs.pending.contains_key(&neighbour),
            invalid
        );
        assert!(stream.light_is_current(key));
        assert_eq!(
            stream.stats().provenance_only_light_jobs,
            before.provenance_only_light_jobs + 1
        );
        assert_eq!(
            stream.stats().light_mesh_invalidations,
            before.light_mesh_invalidations
        );
    }
}

#[test]
fn same_generation_provenance_replacement_uses_current_owner_policy() {
    let position = [0, 5, 7];
    for (captured, current) in [(false, true), (true, false), (false, false)] {
        let (mut stream, key) = packed_stream();
        let neighbour = offset_sub_chunk_key(key, [-1, 0, 0]).unwrap();
        install_current_light(&mut stream, neighbour, 15, 15, false);
        if captured {
            stream.lighting.direct_sky.get_mut(&key).unwrap().mask = Arc::new(direct_bit(position));
        }
        let replacement = stream.lighting.store.light(key).unwrap().as_ref().clone();
        let completion =
            prepare_replacement(&mut stream, key, replacement, DirectSkyMask::Uniform(false));
        let revision = stream.lighting.direct_sky[&key].light_revision;
        stream.lighting.direct_sky.get_mut(&key).unwrap().mask = Arc::new(if current {
            direct_bit(position)
        } else {
            DirectSkyMask::Uniform(false)
        });
        assert_eq!(stream.lighting.direct_sky[&key].light_revision, revision);
        stream
            .lighting
            .waiters
            .entry(key)
            .or_default()
            .insert(neighbour);
        let before = stream.stats();

        let (_, work) = lighting::work::measure(|| stream.accept_light_completion(completion));

        assert!(work[0] > 0, "a different mask must use the live comparison");
        assert_eq!(
            stream.lighting.jobs.pending.contains_key(&neighbour),
            current
        );
        assert_eq!(
            stream.stats().accepted_light_jobs,
            before.accepted_light_jobs + 1
        );
        assert_eq!(
            stream.stats().light_mesh_invalidations,
            before.light_mesh_invalidations
        );
        assert_eq!(
            stream.stats().provenance_only_light_jobs,
            before.provenance_only_light_jobs + u64::from(captured)
        );
        assert_eq!(
            stream.stats().noop_light_jobs,
            before.noop_light_jobs + u64::from(!captured)
        );
    }
}

#[test]
fn same_generation_light_replacement_uses_current_owner_policy() {
    let (mut stream, completion) = packed_completion();
    let previous = stream
        .lighting
        .store
        .light(completion.key)
        .unwrap()
        .as_ref()
        .clone();
    stream
        .lighting
        .store
        .insert_resident(completion.key, previous);
    let accepted = stream.stats().accepted_light_jobs;

    let (_, work) = lighting::work::measure(|| stream.accept_light_completion(completion));

    assert!(
        work[0] > 0,
        "a different light snapshot must use the live comparison"
    );
    assert_eq!(stream.stats().accepted_light_jobs, accepted + 1);
}

#[test]
fn prepared_value_change_preserves_dominance_and_independent_mesh_invalidation() {
    let (mut stream, key) = packed_stream();
    let neighbour = SubChunkKey::new(key.dimension, key.x + 1, key.y, key.z);
    install_current_light(&mut stream, neighbour, 14, 0, false);
    let completion = prepare_replacement(
        &mut stream,
        key,
        SubChunkLight::uniform(15, 0, 0).unwrap(),
        DirectSkyMask::Uniform(false),
    );
    let before = stream.stats();

    let (_, work) = lighting::work::measure(|| stream.accept_light_completion(completion));

    assert_eq!(work, [0, 0]);
    assert!(stream.light_is_current(neighbour));
    assert!(!stream.lighting.jobs.pending.contains_key(&neighbour));
    assert!(stream.mesh_jobs.pending.contains_key(&key));
    assert_eq!(
        stream.stats().value_changed_light_jobs,
        before.value_changed_light_jobs + 1
    );
    assert_eq!(
        stream.stats().light_mesh_invalidations,
        before.light_mesh_invalidations + 1
    );
}

#[test]
fn prepared_faces_do_not_retain_retired_payloads() {
    let light = Arc::new(SubChunkLight::dark(1));
    let direct = Arc::new(direct_bit([0, 5, 7]));
    let retired_light = Arc::downgrade(&light);
    let retired_direct = Arc::downgrade(&direct);
    let (_, prepared) = lighting::results::PreparedMonotonicFaces::prepare(
        Some(light),
        Some(StoredDirectSky {
            light_revision: 1,
            mask: direct,
        }),
        &SubChunkLight::dark(2),
        &DirectSkyMask::Uniform(false),
    );

    assert!(retired_light.upgrade().is_none());
    assert!(retired_direct.upgrade().is_none());
    assert!(prepared.matching(None, None).is_none());
}

#[test]
fn edited_or_evicted_completion_never_consumes_prepared_faces() {
    for evict in [false, true] {
        let (mut stream, completion) = packed_completion();
        if evict {
            stream.evict_column(completion.key.chunk());
        } else {
            stream
                .authority
                .update_block(completion.key, BlockUpdate::new(8, 8, 8, 0, 2), 0)
                .unwrap();
            stream.mark_changed(completion.key, Instant::now());
        }
        let stale = stream.stats().stale_light_jobs;
        let accepted = stream.stats().accepted_light_jobs;

        let (_, work) = lighting::work::measure(|| stream.accept_light_completion(completion));

        assert_eq!(work, [0, 0]);
        assert_eq!(stream.stats().stale_light_jobs, stale + 1);
        assert_eq!(stream.stats().accepted_light_jobs, accepted);
    }
}
