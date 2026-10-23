use super::*;

/// Samples forward movement with full grounded Depth Strider attribute acceleration.
fn wet_speed_frame(sprint: semantic_input::ActionPhase) -> super::super::PhysicsFrameInput {
    let mut frame = speed_frame(sprint);
    frame.facts.depth_strider = 3;
    frame
}

/// Applies the same liquid movement equipment to direct reference ticks.
fn wet_speed_context() -> PhysicsSampleContext {
    PhysicsSampleContext {
        mode_intent: super::super::ModeIntent {
            depth_strider: 3,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Shallow water keeps standing players grounded and prevents swimming entry.
struct ShallowWater;

impl CollisionWorld for ShallowWater {
    fn registry_identity(&self) -> CollisionRegistryIdentity {
        VersionedFloor(1).registry_identity()
    }

    fn collision_boxes(&self, query: Aabb) -> Result<CollisionQuery<Vec<Aabb>>, WorldQueryError> {
        VersionedFloor(1).collision_boxes(query)
    }

    fn block_physics(&self, block: [i32; 3]) -> Result<sim::BlockPhysicsSample, WorldQueryError> {
        let mut sample = VersionedFloor(1).block_physics(block)?;
        if block[1] == 1 {
            sample.layers[0].flags = sim::BlockPhysicsFlags::WATER;
            sample.layers[0].fluid_height_blocks = 0.5;
        }
        Ok(sample)
    }
}

/// A wet jump stop must use the post-stop attribute on that tick and in replay.
#[test]
fn liquid_stop_uses_the_effective_walking_speed_in_the_movement_frame() {
    for modifier in [None, Some(sim::SPRINT_SPEED_MULTIPLIER as f32)] {
        let (mut physics, mut ticker) = walked_physics(0);
        let mut locals = super::super::LocomotionState::default();
        let mut speed = super::super::LocalMovementSpeedAuthority::default();
        let mut effects = super::super::LocalMovementEffectTimeline::default();
        speed.begin_session(7, 0);
        let mut previous = wet_speed_frame(Default::default());
        previous.facts.immobile = true;
        previous.jump.held = true;
        assert!(locals.advance(
            previous,
            &mut physics,
            &mut ticker,
            &mut effects,
            &mut speed,
            &ShallowWater
        ));
        let anchor = ticker.completed_tick();
        assert!(speed.apply(7, 1, 0, attribute(f64::from(0.13_f32), 0.1, modifier)));
        physics.apply_server_movement_flags(0, flags(|flags| flags.sprinting = Some(true)));
        let mut expected = physics.clone();
        let walking_speed = if modifier.is_some() {
            0.1_f32
        } else {
            0.13_f32
        };
        let expected_frame = expected.advance_with_context(
            Duration::from_millis(100),
            MovementInput {
                movement_speed: Some(f64::from(walking_speed)),
                ..physics_movement_input([0.0, 1.0], 180.0, true, false, false, false, None)
            },
            wet_speed_context(),
            &ShallowWater,
        );
        assert_eq!(expected_frame.completed_ticks, 2);
        let mut frame = wet_speed_frame(Default::default());
        frame.delta = Duration::from_millis(100);
        assert!(locals.advance(
            frame,
            &mut physics,
            &mut ticker,
            &mut effects,
            &mut speed,
            &ShallowWater
        ));
        assert!(!physics.sample_at(anchor + 1).unwrap().processed.sprinting);
        assert_eq!(physics.state(), expected.state(), "modifier {modifier:?}");
        assert_eq!(speed.current(), Some(f64::from(walking_speed)));
        let state = physics.state().cloned();
        for _ in 0..2 {
            reconcile_timeline_rewind(&mut ticker, &mut physics, anchor, &ShallowWater).unwrap();
            assert_eq!(
                physics.state(),
                state.as_ref(),
                "replay modifier {modifier:?}"
            );
        }
    }
}

/// A held control restarts on the second catch-up tick and recalculates from defaults.
#[test]
fn a_liquid_stop_and_restart_update_speed_within_one_movement_frame() {
    let (mut physics, mut ticker) = walked_physics(0);
    let mut locals = super::super::LocomotionState::default();
    let mut speed = super::super::LocalMovementSpeedAuthority::default();
    let mut effects = super::super::LocalMovementEffectTimeline::default();
    speed.begin_session(7, 0);
    let mut previous = wet_speed_frame(Default::default());
    previous.facts.immobile = true;
    previous.jump.held = true;
    assert!(locals.advance(
        previous,
        &mut physics,
        &mut ticker,
        &mut effects,
        &mut speed,
        &ShallowWater
    ));
    let anchor = ticker.completed_tick();
    assert!(speed.apply(7, 1, 0, attribute(f64::from(0.26_f32), 0.1, None)));
    physics.apply_server_movement_flags(0, flags(|flags| flags.sprinting = Some(true)));
    let mut expected = physics.clone();
    let input = physics_movement_input([0.0, 1.0], 180.0, true, false, false, false, None);
    expected.advance_with_context(
        Duration::from_millis(50),
        MovementInput {
            movement_speed: Some(f64::from(0.26_f32)),
            ..input
        },
        wet_speed_context(),
        &ShallowWater,
    );
    expected.advance_with_context(
        Duration::from_millis(50),
        MovementInput {
            sprinting: true,
            movement_speed: Some(f64::from(0.1_f32)),
            ..input
        },
        PhysicsSampleContext {
            input: super::super::TickInput {
                sprint_down: true,
                ..Default::default()
            },
            ..wet_speed_context()
        },
        &ShallowWater,
    );
    let mut frame = wet_speed_frame(semantic_input::ActionPhase {
        held: true,
        ..Default::default()
    });
    frame.delta = Duration::from_millis(100);
    assert!(locals.advance(
        frame,
        &mut physics,
        &mut ticker,
        &mut effects,
        &mut speed,
        &ShallowWater
    ));
    assert!(!physics.sample_at(anchor + 1).unwrap().processed.sprinting);
    assert!(physics.sample_at(anchor + 2).unwrap().processed.sprinting);
    assert_eq!(physics.state(), expected.state());
    assert_eq!(
        speed.current(),
        Some(f64::from(0.1_f32 * sim::SPRINT_SPEED_MULTIPLIER as f32))
    );
    let state = physics.state().cloned();
    for _ in 0..2 {
        reconcile_timeline_rewind(&mut ticker, &mut physics, anchor, &ShallowWater).unwrap();
        assert_eq!(physics.state(), state.as_ref());
    }
}

/// Delayed attributes are retained before the stopped tick and survive later replays.
#[test]
fn a_delayed_attribute_replays_the_liquid_speed_edge_and_updates_live_authority() {
    let (mut physics, mut ticker) = walked_physics(0);
    let mut locals = super::super::LocomotionState::default();
    let mut speed = super::super::LocalMovementSpeedAuthority::default();
    let mut effects = super::super::LocalMovementEffectTimeline::default();
    speed.begin_session(7, 0);
    let mut previous = wet_speed_frame(Default::default());
    previous.facts.immobile = true;
    previous.jump.held = true;
    assert!(locals.advance(
        previous,
        &mut physics,
        &mut ticker,
        &mut effects,
        &mut speed,
        &ShallowWater
    ));
    let anchor = ticker.completed_tick();
    assert!(speed.apply(7, 1, 0, attribute(f64::from(0.13_f32), 0.1, None)));
    physics.apply_server_movement_flags(0, flags(|flags| flags.sprinting = Some(true)));
    let mut expected = physics.clone();
    let expected_input = MovementInput {
        movement_speed: Some(f64::from(0.1_f32)),
        ..physics_movement_input([0.0, 1.0], 180.0, true, false, false, false, None)
    };
    expected.advance_with_context(
        Duration::from_millis(50),
        expected_input,
        wet_speed_context(),
        &ShallowWater,
    );
    assert!(locals.advance(
        wet_speed_frame(Default::default()),
        &mut physics,
        &mut ticker,
        &mut effects,
        &mut speed,
        &ShallowWater
    ));
    crate::committed_control::CommittedGameplayState {
        movement: &mut ticker,
        physics: &mut physics,
        effects: &mut effects,
        speed: &mut speed,
        session_generation: 7,
        dimension: 0,
        dimension_transfer_active: false,
    }
    .apply(
        client_world::CommittedControlEvent::LocalMovementSpeed {
            sequence: 2,
            dimension: 0,
            movement: Some(attribute(
                f64::from(0.26_f32),
                0.1,
                Some(sim::SPRINT_SPEED_MULTIPLIER as f32),
            )),
            underwater: None,
            lava: None,
            air_drag_modifier: None,
            tick: anchor,
        },
        &ShallowWater,
        |_| {},
    );
    assert_eq!(physics.state(), expected.state());
    assert_eq!(speed.current(), Some(f64::from(0.1_f32)));
    let state = physics.state().cloned();
    for _ in 0..2 {
        reconcile_timeline_rewind(&mut ticker, &mut physics, anchor, &ShallowWater).unwrap();
        assert_eq!(physics.state(), state.as_ref());
    }
    expected.advance_with_context(
        Duration::from_millis(50),
        expected_input,
        wet_speed_context(),
        &ShallowWater,
    );
    // The next live frame must use the replayed attribute rather than its old current.
    assert!(locals.advance(
        wet_speed_frame(Default::default()),
        &mut physics,
        &mut ticker,
        &mut effects,
        &mut speed,
        &ShallowWater
    ));
    assert_eq!(physics.state(), expected.state());
}

/// A start followed by a liquid stop recalculates speed even when both end flags are clear.
#[test]
fn a_same_tick_sprint_start_and_liquid_stop_apply_both_speed_edges() {
    let (mut physics, mut ticker) = walked_physics(0);
    let mut locals = super::super::LocomotionState::default();
    let mut speed = super::super::LocalMovementSpeedAuthority::default();
    let mut effects = super::super::LocalMovementEffectTimeline::default();
    speed.begin_session(7, 0);
    let mut previous = wet_speed_frame(Default::default());
    previous.facts.immobile = true;
    previous.jump.held = true;
    assert!(locals.advance(
        previous,
        &mut physics,
        &mut ticker,
        &mut effects,
        &mut speed,
        &ShallowWater
    ));
    let anchor = ticker.completed_tick();
    assert!(speed.apply(7, 1, 0, attribute(f64::from(0.26_f32), 0.1, None)));
    let mut expected = physics.clone();
    expected.advance_with_context(
        Duration::from_millis(50),
        MovementInput {
            movement_speed: Some(f64::from(0.1_f32)),
            ..physics_movement_input([0.0, 1.0], 180.0, true, false, false, false, None)
        },
        wet_speed_context(),
        &ShallowWater,
    );
    assert!(locals.advance(
        wet_speed_frame(semantic_input::ActionPhase {
            held: true,
            ..Default::default()
        }),
        &mut physics,
        &mut ticker,
        &mut effects,
        &mut speed,
        &ShallowWater
    ));
    assert!(!physics.sample_at(anchor + 1).unwrap().processed.sprinting);
    assert_eq!(speed.current(), Some(f64::from(0.1_f32)));
    assert_eq!(physics.state(), expected.state());
    reconcile_timeline_rewind(&mut ticker, &mut physics, anchor, &ShallowWater).unwrap();
    assert_eq!(physics.state(), expected.state());
}
