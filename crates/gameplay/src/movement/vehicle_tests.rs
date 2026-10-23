use super::*;
use sim::{
    Aabb, BlockPhysicsFacts, BlockPhysicsFlags, BlockPhysicsSample, CollisionQuery,
    SurfaceResponse, WorldQueryError,
};

/// Still water up to y = 63 over a floor at y = 58.
struct Lake;

impl CollisionWorld for Lake {
    fn collision_boxes(&self, query: Aabb) -> Result<CollisionQuery<Vec<Aabb>>, WorldQueryError> {
        let mut boxes = Vec::new();
        for x in query.min.x.floor() as i32..=query.max.x.floor() as i32 {
            for y in query.min.y.floor() as i32..=query.max.y.floor().min(58.0) as i32 {
                for z in query.min.z.floor() as i32..=query.max.z.floor() as i32 {
                    let min = sim::Vec3::new(f64::from(x), f64::from(y), f64::from(z));
                    boxes.push(Aabb::new(min, min + sim::Vec3::ONE));
                }
            }
        }
        Ok(CollisionQuery::synthetic(boxes))
    }

    fn primary_is_air(
        &self,
        block: [i32; 3],
    ) -> Result<Option<CollisionQuery<bool>>, WorldQueryError> {
        Ok(Some(CollisionQuery::synthetic(block[1] > 62)))
    }

    fn block_physics(&self, block: [i32; 3]) -> Result<BlockPhysicsSample, WorldQueryError> {
        let water = (59..=62).contains(&block[1]);
        Ok(BlockPhysicsSample {
            layers: Box::new([BlockPhysicsFacts {
                friction: 0.6,
                horizontal_speed_factor: 1.0,
                vertical_speed_factor: 1.0,
                fluid_height_blocks: if water { 8.0 / 9.0 } else { 0.0 },
                flags: if water {
                    BlockPhysicsFlags::WATER
                } else {
                    BlockPhysicsFlags::default()
                },
                surface_response: SurfaceResponse::None,
            }]),
            identity: CollisionQuery::synthetic(()).identity,
        })
    }
}

const FORWARD: [f32; 2] = [0.0, 1.0];

fn boat() -> SteeredVehicle {
    SteeredVehicle {
        runtime_id: 7,
        unique_id: -7,
        position: [0.5, 62.6, 0.5],
        yaw: 0.0,
        seat: [0.0, -0.2, 0.0],
        server_moves: 0,
    }
}

/// Steps and commits `ticks` ticks from `first`, returning the last sample.
fn drive(
    prediction: &mut VehiclePrediction,
    vehicle: SteeredVehicle,
    first: u64,
    ticks: u64,
) -> PredictedVehicleSample {
    let mut sample = None;
    for tick in first..first + ticks {
        prediction.step(Some(vehicle), FORWARD, &Lake).unwrap();
        sample = prediction.commit(tick, 32);
    }
    sample.unwrap()
}

#[test]
fn rowing_moves_the_boat_and_seats_the_rider_on_it() {
    let mut prediction = VehiclePrediction::default();
    let first = drive(&mut prediction, boat(), 1, 1);
    assert_eq!(first.unique_id, -7);
    let sample = drive(&mut prediction, boat(), 2, 20);
    assert!(sample.position[0] > first.position[0] + 1.0);
    assert!(sample.velocity[0] > 0.1);
    assert_eq!(sample.rotation, [0.0, 0.0]);

    let seat = prediction.step(Some(boat()), FORWARD, &Lake).unwrap();
    let after = prediction.commit(22, 32).unwrap();
    assert_eq!(seat[0], after.position[0]);
    assert!((seat[1] - (after.position[1] - BOAT_HEIGHT_OFFSET - 0.2)).abs() < 1.0e-6);

    let feed = prediction.feed().unwrap();
    assert_eq!(feed.runtime_id, 7);
    assert_eq!(feed.position[0], after.position[0]);
    assert!((feed.position[1] - (after.position[1] - BOAT_HEIGHT_OFFSET)).abs() < 1.0e-6);
}

#[test]
fn a_correction_rewinds_to_its_tick_and_replays_the_later_paddling() {
    let mut predicted = VehiclePrediction::default();
    drive(&mut predicted, boat(), 1, 10);
    let mut reference = VehiclePrediction::default();
    let corrected_tick = drive(&mut reference, boat(), 1, 4);
    let shifted = VehicleCorrection {
        tick: 4,
        position: [
            corrected_tick.position[0] + 2.0,
            corrected_tick.position[1],
            corrected_tick.position[2],
        ],
        velocity: corrected_tick.velocity,
        rotation: corrected_tick.rotation,
        angular_velocity: None,
        on_ground: false,
    };
    let seat = predicted.correct(&shifted, &Lake).unwrap();
    let before = drive(&mut reference, boat(), 5, 6);
    let after = predicted.feed().unwrap();
    assert!((after.position[0] - (before.position[0] + 2.0)).abs() < 1.0e-3);
    assert_eq!(seat[0], after.position[0]);

    // A correction that agrees, or names an unretained tick, changes nothing.
    assert_eq!(predicted.correct(&shifted, &Lake), None);
    assert_eq!(
        predicted.correct(
            &VehicleCorrection {
                tick: 99,
                ..shifted
            },
            &Lake
        ),
        None
    );
}

#[test]
fn a_server_teleport_restarts_prediction_from_the_new_pose() {
    let mut prediction = VehiclePrediction::default();
    drive(&mut prediction, boat(), 1, 20);
    let teleported = SteeredVehicle {
        position: [40.5, 62.6, 40.5],
        yaw: 90.0,
        server_moves: 1,
        ..boat()
    };
    let sample = drive(&mut prediction, teleported, 21, 1);
    assert!((sample.position[0] - 40.5).abs() < 0.1);
    assert!(sample.rotation[1] >= 90.0);
}

#[test]
fn leaving_the_boat_stops_predicting_it() {
    let mut prediction = VehiclePrediction::default();
    drive(&mut prediction, boat(), 1, 3);
    assert_eq!(prediction.step(None, FORWARD, &Lake), None);
    assert_eq!(prediction.commit(4, 32), None);
    assert_eq!(prediction.feed(), None);
}
