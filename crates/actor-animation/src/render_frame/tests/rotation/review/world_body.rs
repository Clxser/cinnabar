use super::*;

#[test]
fn external_body_preserves_sampled_held_scale_without_committing_the_live_hand() {
    let mut compiled = compiled_fixture(180.0);
    let query = compiled
        .molang_symbols
        .iter()
        .position(|symbol| {
            symbol.kind == MolangSymbolKind::Query
                && symbol.identifier.as_ref() == "query.frame_alpha"
        })
        .unwrap() as u32;
    compiled.rig_bindings[0].held_item_scale = Some(expression(
        &mut compiled,
        &[
            MolangOp::Push(crate::test_support::scalar(1.0)),
            MolangOp::LoadQuery(query),
            MolangOp::Add,
        ],
        2,
    ));
    let context = ActorTickContext {
        is_local: true,
        is_local_player: true,
        is_local_first_person: true,
        ..Default::default()
    };
    let (store, actors) = fixture_with(compiled, 0.0, context);
    let authored = authored_state(&store);
    let tick_scale = store.get(1).unwrap().held_item_scale;
    for alpha in [0.25, 0.75] {
        let mut frame =
            ActorRenderFrame::new(&store, &actors, [0.0; 2], [0.0; 3], alpha).with_world_body(1);
        let body = frame.layers_with_skin(1).unwrap();
        assert_eq!(body.held_item_scale, Some(1.0 + alpha));
        let remaining = frame.remaining_ops();
        assert_eq!(
            frame.layers_with_skin(1).unwrap().held_item_scale,
            body.held_item_scale
        );
        assert_eq!(frame.remaining_ops(), remaining);
        assert_eq!(store.get(1).unwrap().held_item_scale, tick_scale);
        assert_eq!(authored_state(&store), authored);
    }
}

#[test]
fn local_world_body_uses_authoritative_rotation_without_changing_the_hand() {
    let context = ActorTickContext {
        is_local: true,
        is_local_player: true,
        is_local_first_person: true,
        ..Default::default()
    };
    let (mut store, mut actors) = fixture_with(compiled_fixture(180.0), 0.0, context.clone());
    let actor = actors.get_mut(&1).unwrap();
    actor.yaw = 90.0;
    actor.head_yaw = 100.0;
    actor.pitch = 40.0;
    store.advance_tick(&actors, None, Some(1), true, true, |_| context.clone());
    let rig = store.get(1).unwrap();
    let committed = (
        rig.completed_tick,
        rig.previous.to_vec(),
        rig.current.to_vec(),
    );
    let authored = authored_state(&store);
    let hand = sample(&store, &actors[&1], 0.5);
    for alpha in [0.0, 0.25, 0.75, 1.0] {
        let mut frame =
            ActorRenderFrame::new(&store, &actors, [0.0; 2], [0.0; 3], alpha).with_world_body(1);
        let _ = frame.sample_rig_scale(rig);
        let remaining = frame.remaining_ops();
        assert!(remaining < MAX_MOLANG_OPS_PER_RENDER_FRAME);
        let body = frame.layers_with_skin(1).unwrap();
        assert_eq!(
            frame.remaining_ops(),
            remaining,
            "scale and body share one evaluation"
        );
        let expected = [66.25 * alpha, 40.0 * alpha, 33.75 * alpha];
        assert_angles(body.render[0].pose[0].translation_scale, expected);
        assert_angles(body.render[0].previous_pose[0].translation_scale, expected);
        frame.set_remaining_ops(0);
        // Re-entering the same world view retains its actual interpolated local pose.
        frame = frame.with_world_body(1);
        assert_eq!(frame.layers(1).unwrap(), body.render);
        assert_eq!(frame.layers_with_skin(1).unwrap().render, body.render);
        assert_eq!(frame.remaining_ops(), 0);
        assert_eq!(authored_state(&store), authored);
        assert_eq!(sample(&store, &actors[&1], 0.5), hand);
    }
    let retained = store.get(1).unwrap();
    assert_eq!(
        (
            retained.completed_tick,
            retained.previous.to_vec(),
            retained.current.to_vec()
        ),
        committed
    );
}

#[test]
fn additional_view_reuses_remote_samples_and_keeps_its_own_bounded_budget() {
    let (mut store, mut actors) = fixture_with(compiled_fixture(180.0), 0.0, Default::default());
    actors.get_mut(&1).unwrap().head_yaw = 70.0;
    store.advance_tick(&actors, None, None, true, true, |_| Default::default());
    let authored = authored_state(&store);
    let mut live = ActorRenderFrame::new(&store, &actors, [0.0; 2], [0.0; 3], 0.5);
    let remote = live.layers_with_skin(1).unwrap();
    live.set_remaining_ops(0);
    let mut recording = live.into_additional_view().with_world_body(999);
    assert_eq!(recording.remaining_ops(), MAX_MOLANG_OPS_PER_RENDER_FRAME);
    recording.set_remaining_ops(0);
    let reused = recording.layers_with_skin(1).unwrap();
    assert_eq!(reused.render, remote.render);
    assert!(Arc::ptr_eq(&reused.render[0].pose, &remote.render[0].pose));
    assert_eq!(recording.remaining_ops(), 0);
    assert_eq!(authored_state(&store), authored);
}

#[test]
fn additional_view_retries_remote_fallbacks_with_its_fresh_budget() {
    let (reference, actors) = rotated_remote_fixture();
    let mut fresh = ActorRenderFrame::new(&reference, &actors, [0.0; 2], [0.0; 3], 0.5);
    let expected = fresh.layers_with_skin(1).unwrap();
    for remaining in [0, 1] {
        // The reference store must not warm this rig's retained sampling cache.
        let (store, actors) = rotated_remote_fixture();
        let authored = authored_state(&store);
        let mut live = ActorRenderFrame::new(&store, &actors, [0.0; 2], [0.0; 3], 0.5);
        live.set_remaining_ops(remaining);
        let fallback = live.layers_with_skin(1).unwrap();
        assert_ne!(fallback.render[0].pose, expected.render[0].pose);
        let mut recording = live.into_additional_view().with_world_body(999);
        let sampled = recording.layers_with_skin(1).unwrap();
        assert_eq!(sampled.render, expected.render);
        assert!(recording.remaining_ops() < MAX_MOLANG_OPS_PER_RENDER_FRAME);
        assert_eq!(authored_state(&store), authored);
    }
}

#[test]
fn additional_view_reuses_a_retained_complete_sample_after_a_live_fallback() {
    let (store, actors) = rotated_remote_fixture();
    let authored = authored_state(&store);
    let mut fresh = ActorRenderFrame::new(&store, &actors, [0.0; 2], [0.0; 3], 0.5);
    let expected = fresh.layers_with_skin(1).unwrap();
    let mut live = ActorRenderFrame::new(&store, &actors, [0.0; 2], [0.0; 3], 0.5);
    live.set_remaining_ops(0);
    assert_ne!(live.layers_with_skin(1).unwrap().render, expected.render);
    let mut recording = live.into_additional_view().with_world_body(999);
    let sampled = recording.layers_with_skin(1).unwrap();
    assert_eq!(sampled.render, expected.render);
    assert!(Arc::ptr_eq(
        &sampled.render[0].pose,
        &expected.render[0].pose
    ));
    assert_eq!(recording.remaining_ops(), MAX_MOLANG_OPS_PER_RENDER_FRAME);
    assert_eq!(authored_state(&store), authored);
}

/// Prepares changing remote rotation with an untouched frame-sampling cache.
fn rotated_remote_fixture() -> (ActorAnimationStore, HashMap<u64, TestActor>) {
    let (mut store, mut actors) = fixture_with(compiled_fixture(180.0), 0.0, Default::default());
    let actor = actors.get_mut(&1).unwrap();
    actor.yaw = 90.0;
    actor.head_yaw = 100.0;
    actor.pitch = 40.0;
    store.advance_tick(&actors, None, None, true, true, |_| Default::default());
    (store, actors)
}
