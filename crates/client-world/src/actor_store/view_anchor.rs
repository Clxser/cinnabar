//! A supported remote standing-player view uses visual eye facts, never collision-box proportions.

use super::*;

/// Received pose facts identify why a remote view cannot be reconstructed without guessing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlayerViewUnavailable {
    MissingActor,
    NotPlayer,
    InvalidPose,
    Dead,
    Riding,
    Sleeping,
    Crouching,
    Swimming,
    Crawling,
    Gliding,
    Spinning,
    Scaled,
}

const UNSUPPORTED_POSES: &[(ActorFlag, PlayerViewUnavailable)] = &[
    (ActorFlag::Sneaking, PlayerViewUnavailable::Crouching),
    (ActorFlag::Swimming, PlayerViewUnavailable::Swimming),
    (ActorFlag::Crawling, PlayerViewUnavailable::Crawling),
    (ActorFlag::Gliding, PlayerViewUnavailable::Gliding),
    (ActorFlag::DamageNearbyMobs, PlayerViewUnavailable::Spinning),
    (ActorFlag::Sleeping, PlayerViewUnavailable::Sleeping),
    (ActorFlag::Riding, PlayerViewUnavailable::Riding),
];

impl ActorSnapshot {
    /// Samples recorded player head yaw (ordinary actor yaw otherwise) and pitch on their short arcs.
    pub fn interpolated_look(&self, alpha: f32) -> Option<[f32; 2]> {
        if !alpha.is_finite() {
            return None;
        }
        let alpha = alpha.clamp(0.0, 1.0);
        let (previous, current) = if matches!(self.kind, ActorKind::Player { .. }) {
            (self.previous_pose.head_yaw, self.head_yaw)
        } else {
            (self.previous_pose.yaw, self.yaw)
        };
        let yaw = previous + ((current - previous + 180.0).rem_euclid(360.0) - 180.0) * alpha;
        let pitch = self.previous_pose.pitch + (self.pitch - self.previous_pose.pitch) * alpha;
        [yaw, pitch]
            .into_iter()
            .all(f32::is_finite)
            .then_some([yaw, pitch])
    }

    /// Only upright, unscaled and unmounted players have this visual anchor contract.
    /// Pose-transition eye offsets and generic entity eye locations require another native carrier.
    pub(crate) fn recordable_player_view(
        &self,
        alpha: f32,
        riding: bool,
    ) -> Result<([f32; 3], [f32; 2]), PlayerViewUnavailable> {
        use PlayerViewUnavailable as Missing;
        if !matches!(self.kind, ActorKind::Player { .. }) {
            return Err(Missing::NotPlayer);
        }
        if !alpha.is_finite() {
            return Err(Missing::InvalidPose);
        }
        if riding {
            return Err(Missing::Riding);
        }
        if self.is_sleeping() {
            return Err(Missing::Sleeping);
        }
        if self.status.dead {
            return Err(Missing::Dead);
        }
        for &(flag, reason) in UNSUPPORTED_POSES {
            if self.flag(flag.wire_value()) {
                return Err(reason);
            }
        }
        if !matches!(
            self.metadata.get(&ActorDataKey::Scale.wire_value()),
            None | Some(ActorMetadataValue::Float(1.0))
        ) {
            return Err(Missing::Scaled);
        }
        let alpha = alpha.clamp(0.0, 1.0);
        let mut eye = self
            .interpolated_position(alpha)
            .ok_or(Missing::InvalidPose)?;
        eye[1] += protocol::STANDING_PLAYER_EYE_HEIGHT;
        let look = self.interpolated_look(alpha).ok_or(Missing::InvalidPose)?;
        eye.into_iter()
            .all(f32::is_finite)
            .then_some((eye, look))
            .ok_or(Missing::InvalidPose)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Uses the normal player constructor/admission path rather than manufacturing an actor snapshot.
    fn player() -> ActorSnapshot {
        let mut store = ActorStore::new(1, 0);
        store.apply(1, 1, super::super::tests::player_spawn(8, 80, 4.0));
        store.get(8).unwrap().clone()
    }

    #[test]
    fn standing_view_uses_visual_eye_fact_and_interpolated_head_look_without_aabb_scaling() {
        let mut actor = player();
        actor.previous_pose.head_yaw = 179.0;
        actor.head_yaw = -179.0;
        actor.previous_pose.pitch = -20.0;
        actor.pitch = 20.0;
        actor.metadata.insert(
            ActorDataKey::Height.wire_value(),
            ActorMetadataValue::Float(40.0),
        );
        let (eye, look) = actor.recordable_player_view(0.5, false).unwrap();
        let feet = actor.interpolated_position(0.5).unwrap();
        assert_eq!(
            eye,
            [
                feet[0],
                feet[1] + protocol::STANDING_PLAYER_EYE_HEIGHT,
                feet[2]
            ]
        );
        assert_eq!(look, [180.0, 0.0]);
    }

    #[test]
    fn unsupported_poses_scale_rider_links_and_nonplayers_never_receive_a_standing_eye() {
        let actor = player();
        for &(flag, reason) in UNSUPPORTED_POSES {
            let mut posed = actor.clone();
            let bit = flag.wire_value();
            let key = if bit < 64 {
                ActorDataKey::Flags
            } else {
                ActorDataKey::FlagsTwo
            };
            posed.metadata.insert(
                key.wire_value(),
                ActorMetadataValue::Flags(1_u64 << (bit % 64)),
            );
            assert_eq!(posed.recordable_player_view(0.5, false), Err(reason));
        }
        assert_eq!(
            actor.recordable_player_view(0.5, true),
            Err(PlayerViewUnavailable::Riding)
        );
        let mut dead = actor.clone();
        dead.status.dead = true;
        assert_eq!(
            dead.recordable_player_view(0.5, false),
            Err(PlayerViewUnavailable::Dead)
        );
        let mut sleeping = actor.clone();
        sleeping.metadata.insert(
            ActorDataKey::PlayerFlags.wire_value(),
            ActorMetadataValue::Byte(PlayerFlag::Sleeping.wire_value() as i8),
        );
        assert_eq!(
            sleeping.recordable_player_view(0.5, false),
            Err(PlayerViewUnavailable::Sleeping)
        );
        for scale in [0.0, 0.5, 2.0, f32::NAN] {
            let mut scaled = actor.clone();
            scaled.metadata.insert(
                ActorDataKey::Scale.wire_value(),
                ActorMetadataValue::Float(scale),
            );
            assert_eq!(
                scaled.recordable_player_view(0.5, false),
                Err(PlayerViewUnavailable::Scaled)
            );
        }
        let mut entity = actor;
        entity.kind = ActorKind::Entity {
            identifier: "minecraft:player".into(),
        };
        assert_eq!(
            entity.recordable_player_view(0.5, false),
            Err(PlayerViewUnavailable::NotPlayer)
        );
    }
}
