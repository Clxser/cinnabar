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

/// A delayed server sprint write cannot outlast a liquid stop during replay.
/// A held jump in water starts a swim while stopping the sprint. The swim then
/// continues unsprinted until it leaves the water through the side. A server write
/// stamped at the swim start sets sprint for that whole run. Natively the dry
/// swimming tick stops it again, and without a sprint control it stays stopped.
#[test]
fn a_replayed_server_sprint_ends_at_a_liquid_stop() {
    let mut physics = LocalPhysicsController::default();
    physics.reanchor_network_position([0.5, 2.620_01, 2.4], 100, true);
    let mut ticker = MovementTicker::default();
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
        ..MovementInput::default()
    };
    tick(
        &mut physics,
        MovementInput {
            jumping: true,
            ..MovementInput::default()
        },
    );
    let start = tick(
        &mut physics,
        MovementInput {
            jumping: true,
            sprinting: true,
            ..swim
        },
    );
    assert_eq!(start.processed.mode, sim::MovementMode::Swimming);
    assert!(!start.processed.sprinting);
    let mut exited = false;
    for _ in 0..20 {
        let sample = tick(&mut physics, swim);
        assert!(!sample.processed.sprinting);
        if sample.processed.mode != sim::MovementMode::Swimming {
            exited = true;
            break;
        }
    }
    assert!(exited, "the swim leaves the water");
    tick(&mut physics, swim);
    assert_eq!(physics.latest_sneak_sprint(), Some((false, false)));

    let sprint = client_world::MovementFlagUpdate {
        sprinting: Some(true),
        ..Default::default()
    };
    let rewind = physics
        .apply_server_movement_flags(start.tick, sprint)
        .unwrap();
    reconcile_timeline_rewind(&mut ticker, &mut physics, rewind, &SidePool).unwrap();
    assert_eq!(physics.latest_sneak_sprint(), Some((false, false)));
}
