//! Subject eligibility uses captured protocol facts, never labels, skins or a fabricated eye height.

use super::CameraTarget;

macro_rules! target_kinds {
    ($($kind:ident = $code:literal),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub enum CameraTargetKind { $($kind),+ }
        impl CameraTargetKind {
            #[cfg(test)]
            const ALL: &'static [Self] = &[$(Self::$kind),+];
            /// Stable target-track codes are independent of Rust enum discriminants.
            pub const fn code(self) -> u32 { match self { $(Self::$kind => $code),+ } }
            /// Unknown wire codes are malformed; old envelopes explicitly supply Unknown instead.
            pub const fn from_code(code: u32) -> Option<Self> { match code { $($code => Some(Self::$kind),)+ _ => None } }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::ActorPose;
    use std::sync::Arc;

    /// Names and even player-like artwork identifiers are deliberately independent of protocol class.
    fn target(kind: CameraTargetKind) -> CameraTarget {
        CameraTarget {
            runtime_id: 7,
            unique_id: 8,
            spawn_revision: 1,
            name: Arc::from("Player"),
            kind,
            entity_type: Some(Arc::from("minecraft:player")),
            pose: ActorPose {
                position: [0.0; 3],
                velocity: [0.0; 3],
                yaw: 0.0,
                pitch: 0.0,
                roll: 0.0,
                eye_offset: Some([0.0, 1.6, 0.0]),
            },
            teleported: false,
        }
    }

    #[test]
    fn names_eye_offsets_and_identifiers_never_manufacture_a_player() {
        assert!(!target(CameraTargetKind::Entity).is_player_subject(7));
        assert!(!target(CameraTargetKind::Unknown).is_player_subject(9));
        assert!(target(CameraTargetKind::Unknown).is_player_subject(7));
        assert!(target(CameraTargetKind::Player).is_player_subject(0));
        assert!(CameraSubjectFilter::All.includes(&target(CameraTargetKind::Unknown), 9));
        assert!(!CameraSubjectFilter::default().includes(&target(CameraTargetKind::Entity), 9));
    }

    #[test]
    fn classification_codes_and_pose_boundaries_remain_exact() {
        for kind in CameraTargetKind::ALL {
            assert_eq!(CameraTargetKind::from_code(kind.code()), Some(*kind));
        }
        assert_eq!(CameraTargetKind::from_code(u32::MAX), None);
        let original = target(CameraTargetKind::Player);
        assert!(original.can_interpolate(&original));
        let mut changed = original.clone();
        changed.kind = CameraTargetKind::Entity;
        assert!(!original.can_interpolate(&changed));
        changed = original.clone();
        changed.entity_type = Some(Arc::from("minecraft:armor_stand"));
        assert!(!original.can_interpolate(&changed));
    }
}
target_kinds! { Unknown = 0, Player = 1, Entity = 2 }

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CameraSubjectFilter {
    #[default]
    Players,
    All,
}

pub const LEGACY_SUBJECT_NOTICE: &str =
    "This older take has no remote entity types. All entities includes unclassified subjects.";

impl CameraSubjectFilter {
    /// The inspector's explicit advanced mode includes entities and legacy unclassified subjects.
    pub fn includes(self, target: &CameraTarget, local_actor: u64) -> bool {
        self == Self::All || target.is_player_subject(local_actor)
    }

    /// The two choices share one label with viewport and inspector controls.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Players => "Players",
            Self::All => "All entities",
        }
    }

    /// Changing the filter is a view preference and cannot alter an authored camera's bound target.
    pub const fn next(self) -> Self {
        match self {
            Self::Players => Self::All,
            Self::All => Self::Players,
        }
    }
}

impl CameraTarget {
    /// Legacy tracks can prove only the take's explicit local-player identity, never a remote name.
    pub fn is_player_subject(&self, local_actor: u64) -> bool {
        match self.kind {
            CameraTargetKind::Player => true,
            CameraTargetKind::Entity => false,
            CameraTargetKind::Unknown => local_actor != 0 && self.runtime_id == local_actor,
        }
    }

    /// Class and type changes are discrete boundaries even if a producer reuses a spawn identity.
    pub fn can_interpolate(&self, future: &Self) -> bool {
        self.same_lifetime(future)
            && self.kind == future.kind
            && self.entity_type == future.entity_type
            && !future.teleported
    }
}
