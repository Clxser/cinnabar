//! Saved eye provenance distinguishes an exact local presentation from supported remote reconstruction.

use crate::{StudioError, camera::CameraTarget};

macro_rules! views {
    ($($name:ident = $code:literal => $notice:expr),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
        pub enum CameraTargetView {
            #[default]
            Unspecified,
            LocalPresentation,
            StandingPlayer,
            $($name),+
        }
        impl CameraTargetView {
            /// Explicit codes preserve older takes independently of Rust discriminants.
            pub const fn code(self) -> u32 { match self {
                Self::Unspecified => 0, Self::LocalPresentation => 1, Self::StandingPlayer => 2,
                $(Self::$name => $code),+
            } }
            /// Unknown codes are malformed rather than an invented eye policy.
            pub const fn from_code(code: u32) -> Option<Self> { match code {
                0 => Some(Self::Unspecified), 1 => Some(Self::LocalPresentation), 2 => Some(Self::StandingPlayer),
                $($code => Some(Self::$name),)+ _ => None,
            } }
            /// Missing-eye messages describe saved evidence without querying the current connection.
            pub const fn notice(self) -> &'static str { match self {
                Self::Unspecified => "This take has no saved eye-view provenance for this subject at this time.",
                Self::LocalPresentation | Self::StandingPlayer => "This source interval crosses an unsupported eye-view transition.",
                $(Self::$name => $notice),+
            } }
            #[cfg(test)]
            const ALL: &'static [Self] = &[Self::Unspecified, Self::LocalPresentation, Self::StandingPlayer, $(Self::$name),+];
        }
    };
}

views! {
    MissingLocalPresentation = 3 => "The local player's exact eye publication was unavailable in this sample. Centre-based cameras remain available.",
    NotPlayer = 4 => "This entity has no supported recorded eye-view contract. Use Track, Follow or Orbit.",
    InvalidPose = 5 => "The received subject pose cannot provide a finite recorded eye view at this time.",
    Dead = 6 => "A remote death camera is not present in the received eye-view evidence.",
    Riding = 7 => "The received mount/body pose does not include the remote rider's exact eye view.",
    Sleeping = 8 => "The received sleeping body pose does not include a verified remote eye view.",
    Crouching = 9 => "The received crouching pose does not include the remote eye-transition history.",
    Swimming = 10 => "The received swimming body pose does not include a verified remote eye view.",
    Crawling = 11 => "The received crawling body pose does not include a verified remote eye view.",
    Gliding = 12 => "The received gliding body pose does not include a verified remote eye view.",
    Spinning = 13 => "The received spinning body pose does not include a verified remote eye view.",
    Scaled = 14 => "The received scaled body does not include a verified remote eye transform. Collision or head bounds cannot reconstruct it.",
}

impl CameraTarget {
    /// Only actual missing eye evidence produces a completeness notice.
    pub fn eye_view_notice(&self) -> Option<&'static str> {
        self.pose.eye_offset.is_none().then(|| self.view.notice())
    }

    /// A current producer cannot claim exact eyes while omitting them, or attach eyes to an explicit failure.
    pub(in crate::camera) fn validate_view(&self) -> Result<(), StudioError> {
        let consistent = match self.view {
            CameraTargetView::Unspecified => true,
            CameraTargetView::LocalPresentation | CameraTargetView::StandingPlayer => {
                self.pose.eye_offset.is_some()
            }
            _ => self.pose.eye_offset.is_none(),
        };
        if consistent {
            Ok(())
        } else {
            Err(StudioError::Invalid("inconsistent camera eye provenance"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_codes_and_evidence_validation_are_complete() {
        let mut target = super::super::tests::target(crate::camera::CameraTargetKind::Player);
        for &view in CameraTargetView::ALL {
            assert_eq!(CameraTargetView::from_code(view.code()), Some(view));
            target.view = view;
            target.pose.eye_offset = Some([0.0, 1.62, 0.0]);
            let available = matches!(
                view,
                CameraTargetView::Unspecified
                    | CameraTargetView::LocalPresentation
                    | CameraTargetView::StandingPlayer
            );
            assert_eq!(target.validate().is_ok(), available);
            assert!(target.eye_view_notice().is_none());
            target.pose.eye_offset = None;
            assert_eq!(
                target.validate().is_ok(),
                !matches!(
                    view,
                    CameraTargetView::LocalPresentation | CameraTargetView::StandingPlayer
                )
            );
            assert!(target.eye_view_notice().is_some());
        }
        assert!(CameraTargetView::from_code(u32::MAX).is_none());
    }
}
