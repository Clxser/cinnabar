//! Captures compact target anchors from received actors without preparing another animation/render list.

use std::sync::{Arc, OnceLock};

use bevy::prelude::*;
use client_presentation::{
    actor_publication::ActorFrameState, local_player::LocalPlayerFrameCarrier,
};
use recording_scene::targets::TargetCapture;
use recording_studio::{
    MediaTime,
    camera::{
        ActorPose, CameraTarget, CameraTargetKind, CameraTargetView, MAX_CAMERA_TARGETS,
        MAX_TARGET_NAME_BYTES,
    },
    capture::CaptureSession,
};

use super::{Active, ClientWorld};

pub(super) struct NativeTargets {
    pub capture: TargetCapture,
    last_frame: u64,
    scratch: Vec<CameraTarget>,
}

/// The zero-time target set includes the local subject even when its first-person body is hidden.
pub(super) fn start(world: &World, session: &CaptureSession) -> Result<NativeTargets, String> {
    let mut capture = TargetCapture::start_with_disk(
        session.directory(),
        session.info().clone(),
        session.disk_budget().clone(),
    )?;
    let mut baseline = Vec::new();
    snapshot(world, &mut baseline)?;
    let scratch = capture.sample(MediaTime::ZERO, baseline)?;
    Ok(NativeTargets {
        capture,
        last_frame: 0,
        scratch,
    })
}

/// Capture cadence shares the video's clock; actor naming, positions and lifetime facts remain native.
pub(super) fn sample(world: &World, active: &mut Active, at: MediaTime) -> Result<(), String> {
    let frame = active
        .session
        .settings()
        .fps
        .frame_at(at)
        .map_err(|error| error.to_string())?;
    let Some(targets) = &mut active.targets else {
        return Ok(());
    };
    if targets.last_frame == frame {
        return Ok(());
    }
    snapshot(world, &mut targets.scratch)?;
    targets.scratch = targets
        .capture
        .sample(at, std::mem::take(&mut targets.scratch))?;
    targets.last_frame = frame;
    Ok(())
}

/// Local published eyes and supported standing remote-player eyes are retained separately from centres.
fn snapshot(world: &World, targets: &mut Vec<CameraTarget>) -> Result<(), String> {
    let stream = world
        .get_resource::<ClientWorld>()
        .and_then(|world| world.stream.as_ref())
        .ok_or("The recorded actor authority is unavailable.")?;
    let authority = stream.authority();
    if authority.actor_count() > MAX_CAMERA_TARGETS {
        return Err("The world exceeds the camera target recording budget.".into());
    }
    let alpha = world
        .get_resource::<ActorFrameState>()
        .and_then(ActorFrameState::frame_step)
        .map_or(0.0, |step| step.partial_tick);
    targets.clear();
    targets.reserve(authority.actor_count().saturating_add(1));
    for actor in authority.remote_actors() {
        if let Some(target) = received_target(authority, actor, alpha, false) {
            targets.push(target);
        }
    }
    if let Some(local) = world
        .get_resource::<LocalPlayerFrameCarrier>()
        .and_then(LocalPlayerFrameCarrier::snapshot)
        .filter(|local| local.actor_session_id() == authority.actor_session_id())
    {
        let runtime_id = stream.local_player_runtime_id();
        let actor = authority.actor(runtime_id);
        let direction = local.direction();
        let [yaw, pitch, roll] = super::observations::angles(direction, local.rotation() * Vec3::Y);
        targets.push(CameraTarget {
            runtime_id,
            unique_id: authority.local_player_unique_id(),
            spawn_revision: actor.map_or(0, |actor| actor.spawn_revision),
            kind: CameraTargetKind::Player,
            entity_type: Some(player_type()),
            view: CameraTargetView::LocalPresentation,
            name: label(
                authority.actor_display_name(authority.local_player_unique_id()),
                runtime_id,
            ),
            pose: ActorPose {
                position: local.feet().to_array().map(f64::from),
                velocity: [0.0; 3],
                yaw,
                pitch,
                roll,
                eye_offset: Some((local.eye() - local.feet()).to_array().map(f64::from)),
            },
            teleported: actor.is_some_and(|actor| actor.teleported),
        });
    } else if let Some(actor) = authority.actor(stream.local_player_runtime_id())
        && let Some(target) = received_target(authority, actor, alpha, true)
    {
        targets.push(target);
    }
    if targets.len() > MAX_CAMERA_TARGETS {
        return Err("The world exceeds the camera target recording budget.".into());
    }
    Ok(())
}

/// Missing local eye publication cannot erase a received subject usable by centre-based cameras.
fn received_target(
    authority: &client_world::WorldAuthority,
    actor: &client_world::ActorSnapshot,
    alpha: f32,
    local: bool,
) -> Option<CameraTarget> {
    let position = actor.interpolated_position(alpha)?;
    let anchor = if local {
        position
    } else {
        actor
            .bounding_box_at(position)
            .map_or(position, |(min, max)| {
                std::array::from_fn(|axis| (min[axis] + max[axis]) * 0.5)
            })
    };
    let native_view = authority.recordable_player_view(actor.runtime_id, alpha);
    let view = if local {
        CameraTargetView::MissingLocalPresentation
    } else {
        native_view.map_or_else(view_unavailable, |_| CameraTargetView::StandingPlayer)
    };
    let eye = (!local).then_some(native_view.ok()).flatten();
    let [yaw, pitch] = eye.map_or_else(
        || {
            actor
                .interpolated_look(alpha)
                .unwrap_or([actor.yaw, actor.pitch])
        },
        |(_, look)| look,
    );
    Some(CameraTarget {
        runtime_id: actor.runtime_id,
        unique_id: actor.unique_id,
        spawn_revision: actor.spawn_revision,
        kind: match &actor.kind {
            protocol::ActorKind::Player { .. } => CameraTargetKind::Player,
            protocol::ActorKind::Entity { .. } => CameraTargetKind::Entity,
        },
        entity_type: Some(match &actor.kind {
            protocol::ActorKind::Player { .. } => player_type(),
            protocol::ActorKind::Entity { identifier } => Arc::clone(identifier),
        }),
        view,
        name: label(
            authority.actor_display_name(actor.unique_id),
            actor.runtime_id,
        ),
        pose: ActorPose {
            position: anchor.map(f64::from),
            velocity: [0.0; 3],
            yaw: f64::from(yaw),
            pitch: f64::from(pitch),
            roll: 0.0,
            eye_offset: eye
                .map(|(eye, _)| std::array::from_fn(|axis| f64::from(eye[axis] - anchor[axis]))),
        },
        teleported: actor.teleported,
    })
}

/// Every unsupported source fact remains explicit in the portable target track.
fn view_unavailable(reason: client_world::PlayerViewUnavailable) -> CameraTargetView {
    use client_world::PlayerViewUnavailable as Missing;
    match reason {
        Missing::MissingActor | Missing::InvalidPose => CameraTargetView::InvalidPose,
        Missing::NotPlayer => CameraTargetView::NotPlayer,
        Missing::Dead => CameraTargetView::Dead,
        Missing::Riding => CameraTargetView::Riding,
        Missing::Sleeping => CameraTargetView::Sleeping,
        Missing::Crouching => CameraTargetView::Crouching,
        Missing::Swimming => CameraTargetView::Swimming,
        Missing::Crawling => CameraTargetView::Crawling,
        Missing::Gliding => CameraTargetView::Gliding,
        Missing::Spinning => CameraTargetView::Spinning,
        Missing::Scaled => CameraTargetView::Scaled,
    }
}

#[cfg(test)]
mod tests;

/// One immutable canonical player type avoids allocating an identical label per actor and capture frame.
fn player_type() -> Arc<str> {
    static TYPE: OnceLock<Arc<str>> = OnceLock::new();
    Arc::clone(TYPE.get_or_init(|| {
        Arc::from(
            assets::facts::entity::NativeEntityClass::Player
                .identifier()
                .expect("native player identifier"),
        )
    }))
}

const _: () = assert!(MAX_TARGET_NAME_BYTES >= protocol::MAX_ACTOR_IDENTIFIER_BYTES);

/// Names stay shared when already bounded; unusually long labels stop at a UTF-8 boundary.
fn label(name: Option<Arc<str>>, runtime_id: u64) -> Arc<str> {
    let name = name.unwrap_or_else(|| Arc::from(format!("Actor {runtime_id}")));
    if name.len() <= MAX_TARGET_NAME_BYTES {
        return name;
    }
    let mut end = MAX_TARGET_NAME_BYTES;
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    Arc::from(&name[..end])
}
