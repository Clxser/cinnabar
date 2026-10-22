//! Replay body masking depends only on the final camera and its admitted source sample.

use recording_scene::targets::TargetFrame;
use recording_studio::camera::{CameraPose, CameraTarget, CameraTargetRef};

/// An editor near-eye guard, not a reconstructed intersection with arbitrary animated skin geometry.
const LOCAL_EYE_GUARD_BLOCKS: f64 = 0.4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ActorVisibility {
    local: u64,
    recorded_hide_local: bool,
    near_eye: Option<CameraTargetRef>,
    spectated: Option<CameraTargetRef>,
}

impl ActorVisibility {
    /// Custom views keep their own HUD/hand policy even when they occupy the recorded native eye.
    pub(super) fn new(
        local_actor: u64,
        source_body_visible: Option<bool>,
        targets: Option<&TargetFrame>,
        pose: CameraPose,
        recorded_view: bool,
        spectated: Option<CameraTargetRef>,
    ) -> Self {
        let local = targets.and_then(|targets| targets.target(local_actor));
        let spectated = spectated.and_then(|reference| {
            let target = targets?.target(reference.runtime_id)?;
            target.matches(reference).then(|| target.reference())
        });
        Self {
            local: local_actor,
            recorded_hide_local: recorded_view && source_body_visible == Some(false),
            near_eye: (!recorded_view).then(|| near_eye(local, pose)).flatten(),
            spectated,
        }
    }

    /// Body, equipment and skin layers share one spawn mask; reused IDs do not inherit custom masks.
    pub(super) fn visible(&self, runtime: u64, spawn: u64) -> bool {
        !(self.recorded_hide_local && runtime == self.local)
            && ![self.near_eye, self.spectated]
                .into_iter()
                .flatten()
                .any(|target| {
                    target.runtime_id == runtime
                        && target
                            .lifetime
                            .is_some_and(|lifetime| lifetime.spawn_revision == spawn)
                })
    }
}

/// Missing legacy eyes do not acquire a fabricated height; rotation and lens edits cannot reveal a face.
fn near_eye(local: Option<&CameraTarget>, pose: CameraPose) -> Option<CameraTargetRef> {
    let local = local?;
    let offset = local.pose.eye_offset?;
    let squared: f64 = (0..3)
        .map(|axis| (pose.position[axis] - local.pose.position[axis] - offset[axis]).powi(2))
        .sum();
    (squared <= LOCAL_EYE_GUARD_BLOCKS * LOCAL_EYE_GUARD_BLOCKS).then(|| local.reference())
}

#[cfg(test)]
mod tests {
    use super::*;
    use recording_studio::camera::{ActorPose, CameraTargetKind, TargetLifetime};
    use std::sync::Arc;

    /// Explicit source eyes and lifetime identity are sufficient; no renderer or live authority is needed.
    fn local() -> CameraTarget {
        CameraTarget {
            runtime_id: 7,
            unique_id: 10,
            spawn_revision: 3,
            name: Arc::from("Local fixture"),
            kind: CameraTargetKind::Player,
            entity_type: Some(Arc::from("minecraft:player")),
            pose: ActorPose {
                position: [0.5, -60.0, 0.5],
                velocity: [0.0; 3],
                yaw: 0.0,
                pitch: 0.0,
                roll: 0.0,
                eye_offset: Some([0.0, 1.62, 0.0]),
            },
            teleported: false,
        }
    }

    /// Target publications use the same sorted, source-time identity lookup as production playback.
    fn targets(target: &CameraTarget) -> TargetFrame {
        TargetFrame {
            at: recording_studio::MediaTime::ZERO,
            targets: vec![target.clone()].into(),
        }
    }

    #[test]
    fn free_camera_at_native_eye_hides_its_body_for_every_rotation_and_lens() {
        let local = local();
        let targets = targets(&local);
        for (yaw, pitch, roll, fov) in [(0.0, 0.0, 0.0, 66.0), (180.0, 80.0, 25.0, 120.0)] {
            let pose = CameraPose {
                position: local.pose.centre(),
                yaw,
                pitch,
                roll,
                fov,
            };
            assert_eq!(near_eye(Some(&local), pose), Some(local.reference()));
            let hidden = ActorVisibility::new(
                local.runtime_id,
                Some(false),
                Some(&targets),
                pose,
                false,
                None,
            );
            assert!(!hidden.visible(local.runtime_id, local.spawn_revision));
            assert!(hidden.visible(local.runtime_id, local.spawn_revision + 1));
            assert!(hidden.visible(8, local.spawn_revision));
        }
    }

    #[test]
    fn leaving_the_eye_restores_the_body_without_history_or_seeking_dependence() {
        let local = local();
        let targets = targets(&local);
        for displacement in [2.0, 0.0, 0.5, 1.0, 1.000001, 0.0, 2.0] {
            let mut pose = CameraPose {
                position: local.pose.centre(),
                ..Default::default()
            };
            pose.position[0] += LOCAL_EYE_GUARD_BLOCKS * displacement;
            let mask = near_eye(Some(&local), pose);
            assert_eq!(mask.is_some(), displacement <= 1.0);
            let visibility = ActorVisibility::new(
                local.runtime_id,
                Some(false),
                Some(&targets),
                pose,
                false,
                None,
            );
            assert_eq!(
                visibility.visible(local.runtime_id, local.spawn_revision),
                displacement > 1.0
            );
        }
    }

    #[test]
    fn missing_native_eyes_and_invalid_camera_positions_do_not_manufacture_a_mask() {
        let mut local = local();
        let pose = CameraPose {
            position: local.pose.centre(),
            ..Default::default()
        };
        assert!(near_eye(None, pose).is_none());
        local.pose.eye_offset = None;
        assert!(near_eye(Some(&local), pose).is_none());
        local.pose.eye_offset = Some([0.0, 1.62, 0.0]);
        assert!(
            near_eye(
                Some(&local),
                CameraPose {
                    position: [f64::NAN; 3],
                    ..pose
                }
            )
            .is_none()
        );
    }

    #[test]
    fn recorded_third_person_and_explicit_spectator_masks_remain_independent() {
        let local = local();
        let targets = targets(&local);
        let pose = CameraPose {
            position: local.pose.centre(),
            ..Default::default()
        };
        let target = CameraTargetRef {
            runtime_id: 7,
            lifetime: Some(TargetLifetime {
                unique_id: 10,
                spawn_revision: 3,
            }),
        };
        let visibility =
            ActorVisibility::new(99, Some(true), Some(&targets), pose, true, Some(target));
        assert!(!visibility.visible(7, 3));
        assert!(visibility.visible(7, 4));
        assert!(visibility.visible(8, 3));
        assert!(visibility.visible(99, 1));
        let visibility = ActorVisibility::new(99, Some(false), Some(&targets), pose, true, None);
        assert!(!visibility.visible(99, 1));
        let visibility = ActorVisibility::new(
            local.runtime_id,
            Some(true),
            Some(&targets),
            pose,
            true,
            None,
        );
        assert!(
            visibility.visible(local.runtime_id, local.spawn_revision),
            "recorded third person remains visible even at its native eye"
        );
    }

    #[test]
    fn stale_or_missing_subjects_never_mask_unrelated_actor_lifetimes() {
        let local = local();
        let targets = targets(&local);
        let pose = CameraPose {
            position: [20.0; 3],
            ..Default::default()
        };
        for reference in [
            CameraTargetRef {
                runtime_id: local.runtime_id,
                lifetime: Some(TargetLifetime {
                    unique_id: local.unique_id + 1,
                    spawn_revision: local.spawn_revision,
                }),
            },
            CameraTargetRef {
                runtime_id: local.runtime_id,
                lifetime: Some(TargetLifetime {
                    unique_id: local.unique_id,
                    spawn_revision: local.spawn_revision + 1,
                }),
            },
            CameraTargetRef {
                runtime_id: 99,
                lifetime: None,
            },
        ] {
            let visibility = ActorVisibility::new(
                local.runtime_id,
                Some(false),
                Some(&targets),
                pose,
                false,
                Some(reference),
            );
            assert!(visibility.visible(local.runtime_id, local.spawn_revision));
        }
        let visibility = ActorVisibility::new(
            local.runtime_id,
            Some(false),
            None,
            CameraPose {
                position: local.pose.centre(),
                ..pose
            },
            false,
            None,
        );
        assert!(visibility.visible(local.runtime_id, local.spawn_revision));
    }
}
