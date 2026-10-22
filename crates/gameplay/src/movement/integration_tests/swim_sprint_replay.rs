use super::*;

/// Water three blocks deep above the floor for `z < 3`; dry floor beyond.
struct SidePool;

impl SidePool {
    fn water(block: [i32; 3]) -> bool {
        (1..4).contains(&block[1]) && block[2] < 3
    }
}

impl CollisionWorld for SidePool {
    fn registry_identity(&self) -> CollisionRegistryIdentity {
        VersionedFloor(1).registry_identity()
    }

    fn collision_boxes(&self, query: Aabb) -> Result<CollisionQuery<Vec<Aabb>>, WorldQueryError> {
        VersionedFloor(1).collision_boxes(query)
    }

    fn block_physics(&self, block: [i32; 3]) -> Result<sim::BlockPhysicsSample, WorldQueryError> {
        let mut sample = VersionedFloor(1).block_physics(block)?;
        if Self::water(block) {
            sample.layers[0].flags = sim::BlockPhysicsFlags::from_bits(
                sample.layers[0].flags.bits() | sim::BlockPhysicsFlags::WATER.bits(),
            )
            .unwrap();
            sample.layers[0].fluid_height_blocks = 1.0;
        }
        Ok(sample)
    }

    fn primary_is_air(
        &self,
        block: [i32; 3],
    ) -> Result<Option<CollisionQuery<bool>>, WorldQueryError> {
        Ok(Some(CollisionQuery {
            value: block[1] >= 1 && !Self::water(block),
            identity: self.block_physics(block)?.identity,
        }))
    }
}

const SWIM_TICKS: usize = 18;

/// Sprints into a held-jump swim start in water, which ends the sprint, then swims
/// unsprinted out through the side. Returns the swim-start tick; `on_time`
/// applies a server sprint write right after it.
fn swim_out_of_the_side(
    physics: &mut LocalPhysicsController,
    ticker: &mut MovementTicker,
    on_time: Option<client_world::MovementFlagUpdate>,
) -> u64 {
    physics.reanchor_network_position([0.5, 2.620_01, 2.4], 100, true);
    ticker.reset(7, 100, [0.5, 2.620_01, 2.4]);
    ticker.set_source(MovementSource::Physics);
    let mut tick = |physics: &mut LocalPhysicsController, input: MovementInput| {
        let frame = physics.advance(Duration::from_millis(50), input, &SidePool);
        assert!(frame.blocked.is_none(), "blocked: {:?}", frame.blocked);
        let sample = frame.samples[0].clone();
        ticker.enqueue_completed_physics(sample.clone()).unwrap();
        sample
    };
    let swim = MovementInput {
        forward: 1.0,
        movement_speed: Some(0.1),
        ..MovementInput::default()
    };
    tick(
        physics,
        MovementInput {
            jumping: true,
            ..MovementInput::default()
        },
    );
    let start = tick(
        physics,
        MovementInput {
            jumping: true,
            sprinting: true,
            ..swim
        },
    );
    assert_eq!(start.processed.mode, sim::MovementMode::Swimming);
    assert!(!start.processed.sprinting);
    if let Some(flags) = on_time {
        assert_eq!(physics.apply_server_movement_flags(start.tick, flags), None);
    }
    let modes: Vec<_> = (0..SWIM_TICKS)
        .map(|_| tick(physics, swim).processed.mode)
        .collect();
    assert_eq!(
        modes.last(),
        Some(&sim::MovementMode::Walking),
        "the swim leaves the water"
    );
    start.tick
}

/// A delayed server sprint write cannot outlast a liquid stop during replay.
/// Natively the flag is applied once at its tick: the swim keeps it in water, the
/// dry swimming tick stops it, and without a sprint control it stays stopped.
/// Replay must match the same write delivered on time, including walking speed,
/// and must not leave the stopped sprint pending for the live controls.
#[test]
fn a_replayed_server_sprint_ends_at_a_liquid_stop() {
    let sprint = client_world::MovementFlagUpdate {
        sprinting: Some(true),
        ..Default::default()
    };
    let mut on_time = LocalPhysicsController::default();
    swim_out_of_the_side(&mut on_time, &mut MovementTicker::default(), Some(sprint));

    let mut delayed = LocalPhysicsController::default();
    let mut ticker = MovementTicker::default();
    let start = swim_out_of_the_side(&mut delayed, &mut ticker, None);
    assert_eq!(delayed.latest_sneak_sprint(), Some((false, false)));
    let rewind = delayed.apply_server_movement_flags(start, sprint).unwrap();
    reconcile_timeline_rewind(&mut ticker, &mut delayed, rewind, &SidePool).unwrap();
    assert_eq!(delayed.state(), on_time.state());
    assert_eq!(delayed.latest_sneak_sprint(), Some((false, false)));
    assert_eq!(
        delayed
            .take_server_control_flags()
            .and_then(|flags| flags.sprinting),
        None
    );
}

/// A later server sprint write, stamped after the replayed liquid stop, still applies.
#[test]
fn a_server_sprint_after_a_replayed_liquid_stop_still_applies() {
    let sprint = client_world::MovementFlagUpdate {
        sprinting: Some(true),
        ..Default::default()
    };
    let mut physics = LocalPhysicsController::default();
    let mut ticker = MovementTicker::default();
    let start = swim_out_of_the_side(&mut physics, &mut ticker, None);
    let rewind = physics.apply_server_movement_flags(start, sprint).unwrap();
    reconcile_timeline_rewind(&mut ticker, &mut physics, rewind, &SidePool).unwrap();
    assert_eq!(physics.latest_sneak_sprint(), Some((false, false)));

    let later = physics.state().unwrap().tick - 2;
    let rewind = physics.apply_server_movement_flags(later, sprint).unwrap();
    reconcile_timeline_rewind(&mut ticker, &mut physics, rewind, &SidePool).unwrap();
    assert_eq!(physics.latest_sneak_sprint(), Some((false, true)));
}

/// A position correction that moves a latched sprinting swimmer out of the water
/// stops the sprint on the first replayed tick; the stale latched request recorded
/// by later ticks cannot restart it without a sprint control.
#[test]
fn a_correction_out_of_the_water_stops_the_replayed_sprint() {
    let mut physics = LocalPhysicsController::default();
    let mut ticker = MovementTicker::default();
    physics.reanchor_network_position([0.5, 2.620_01, 0.5], 100, true);
    ticker.reset(7, 100, [0.5, 2.620_01, 0.5]);
    ticker.set_source(MovementSource::Physics);
    let swim = MovementInput {
        forward: 1.0,
        sprinting: true,
        ..MovementInput::default()
    };
    let mut samples = Vec::new();
    for _ in 0..6 {
        let frame = physics.advance(Duration::from_millis(50), swim, &SidePool);
        let sample = frame.samples[0].clone();
        ticker.enqueue_completed_physics(sample.clone()).unwrap();
        samples.push(sample);
    }
    assert!(samples[1..].iter().all(|sample| {
        sample.processed.mode == sim::MovementMode::Swimming && sample.processed.sprinting
    }));
    let corrected = samples[2].clone();
    reconcile_candidate_physics_correction(
        &mut ticker,
        &mut physics,
        [0.5, corrected.position[1], 3.6],
        corrected.tick,
        false,
        PhysicsCorrectionMode::ReplayIfRetained,
        &SidePool,
    )
    .unwrap();
    assert_eq!(physics.mode(), sim::MovementMode::Walking);
    assert_eq!(physics.latest_sneak_sprint(), Some((false, false)));
}

/// An ordinary input-rejection stop also ends a replayed server sprint: walking
/// with too little forward input keeps the native stop request after the flag.
#[test]
fn a_replayed_server_sprint_ends_at_an_input_stop() {
    let (mut physics, mut ticker) = walked_physics(0);
    let slow = MovementInput {
        forward: 0.5,
        ..MovementInput::default()
    };
    let mut ticks = Vec::new();
    for _ in 0..4 {
        let frame = physics.advance(Duration::from_millis(50), slow, &VersionedFloor(1));
        let sample = frame.samples[0].clone();
        ticker.enqueue_completed_physics(sample.clone()).unwrap();
        ticks.push(sample.tick);
    }
    let sprint = client_world::MovementFlagUpdate {
        sprinting: Some(true),
        ..Default::default()
    };
    let rewind = physics
        .apply_server_movement_flags(ticks[0], sprint)
        .unwrap();
    reconcile_timeline_rewind(&mut ticker, &mut physics, rewind, &VersionedFloor(1)).unwrap();
    assert_eq!(physics.latest_sneak_sprint(), Some((false, false)));
    assert_eq!(
        physics
            .take_server_control_flags()
            .and_then(|flags| flags.sprinting),
        None
    );
}
