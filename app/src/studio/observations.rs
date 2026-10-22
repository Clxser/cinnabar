//! Borrows committed gameplay evidence without consuming the audio, particle or networking queues.

use super::*;
use recording_studio::{automation::GameplayEventKind, camera::CameraPose};

/// Session and dimension changes terminate a POV take before their combat histories can mix.
pub(super) fn identity(world: &World) -> Option<(u64, i32, u64)> {
    let stream = world.get_resource::<ClientWorld>()?.stream.as_ref()?;
    let authority = stream.authority();
    Some((
        authority.actor_session_id(),
        authority.current_dimension(),
        stream.local_player_runtime_id(),
    ))
}

/// Uses the menu's actual scene history when choosing Resume versus world selection.
pub(super) fn identity_from_menu(menu: &MenuRuntime) -> bool {
    menu.over_world() && !menu.is_connecting()
}

/// Baselines the current admitted attack so starting a take cannot replay an old intent.
pub(super) fn attack_revision(world: &World) -> u64 {
    world
        .get_resource::<client_interaction::melee::MeleeRuntime>()
        .and_then(|melee| melee.admitted_attack())
        .map_or(0, |attack| attack.sequence)
}

/// Baselines bounded server notices without copying the retained actor store.
pub(super) fn combat_revision(world: &World) -> u64 {
    world
        .get_resource::<ClientWorld>()
        .and_then(|world| world.stream.as_ref())
        .and_then(|stream| stream.authority().combat_evidence().last())
        .map_or(0, |notice| notice.revision)
}

/// Local FIFO admission is an intent; unattributed server hurt/death stays correlated or observed.
pub(super) fn capture(world: &World, active: &mut Active, at: MediaTime) -> Result<(), String> {
    if let Some(attack) = world
        .get_resource::<client_interaction::melee::MeleeRuntime>()
        .and_then(|melee| melee.admitted_attack())
        && attack.sequence > active.last_attack
    {
        active.session.event(
            at,
            GameplayEventKind::AttackIntent {
                target: attack.hit.runtime_id,
            },
        )?;
        active.last_attack = attack.sequence;
    }
    let Some(stream) = world
        .get_resource::<ClientWorld>()
        .and_then(|world| world.stream.as_ref())
    else {
        return Ok(());
    };
    for notice in stream.authority().combat_evidence() {
        if notice.revision <= active.last_combat {
            continue;
        }
        let missed = notice
            .revision
            .saturating_sub(active.last_combat)
            .saturating_sub(1);
        if missed != 0 {
            active
                .session
                .missed_observations(missed, active.identity.2);
        }
        active.last_combat = notice.revision;
        if notice.dimension != active.identity.1 {
            continue;
        }
        let kind = match notice.kind {
            client_world::ActorCombatKind::Hurt => GameplayEventKind::Hurt {
                victim: notice.runtime_id,
                attacker: None,
                player: notice.player,
            },
            client_world::ActorCombatKind::Death => GameplayEventKind::Death {
                victim: notice.runtime_id,
                attacker: None,
                player: notice.player,
            },
        };
        active.session.event(at, kind)?;
    }
    Ok(())
}

/// Stores the actual final camera view in the editor's degree-based Bedrock coordinate convention.
pub(super) fn camera(world: &mut World) -> Option<CameraPose> {
    let mut query = world.query_filtered::<(&GlobalTransform, &Projection), With<client_presentation::camera::FlyCamera>>();
    let (transform, projection) = query.single(world).ok()?;
    let Projection::Perspective(projection) = projection else {
        return None;
    };
    let forward = *transform.forward();
    let up = *transform.up();
    let [yaw, pitch, roll] = angles(forward, up);
    let pose = CameraPose {
        position: transform.translation().to_array().map(f64::from),
        yaw,
        pitch,
        roll,
        fov: f64::from(projection.fov.to_degrees()),
    };
    pose.validate().ok().map(|()| pose)
}

/// Shares the native camera-axis convention between the visible view and a local recorded subject.
pub(super) fn angles(forward: Vec3, up: Vec3) -> [f64; 3] {
    let right = forward.cross(Vec3::Y).normalize_or_zero();
    let reference_up = right.cross(forward).normalize_or_zero();
    [
        f64::from((-forward.x).atan2(forward.z).to_degrees()),
        f64::from((-forward.y).atan2(forward.x.hypot(forward.z)).to_degrees()),
        f64::from(up.dot(right).atan2(up.dot(reference_up)).to_degrees()),
    ]
}
