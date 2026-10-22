//! Portable navigation intents own both physical routing policy and contextual help.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CameraNavigationAction {
    Speed(i8),
    ResetSpeed,
    ToggleSmooth,
    Level,
}

macro_rules! gestures {
    ($($variant:ident => $chord:literal, $description:literal);+ $(;)?) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub enum NavigationGesture { $($variant),+ }
        impl NavigationGesture {
            pub const ALL: &[Self] = &[$(Self::$variant),+];
            /// Chords shown in help come from the same portable gesture kind used by input.
            pub const fn chord(self) -> &'static str { match self { $(Self::$variant => $chord),+ } }
            pub const fn description(self) -> &'static str { match self { $(Self::$variant => $description),+ } }
            /// A static combined hint avoids allocating text while an inspection view moves.
            pub const fn hint(self) -> &'static str { match self { $(Self::$variant => concat!($chord, ": ", $description)),+ } }
        }
    };
}

gestures! {
    Orbit => "Left drag", "Orbit around the selected player or forward pivot";
    Look => "Right drag", "Look around without moving the camera";
    Pan => "Middle drag", "Pan relative to the view, including camera roll";
    Move => "WASD / Q E", "Fly horizontally / move down and up";
    Fast => "Shift", "Fly faster using the current speed multiplier";
    Dolly => "Wheel", "Dolly forward and backward without changing the lens";
    Zoom => "Ctrl/Cmd + wheel", "Change optical zoom without moving the camera";
    Speed => "Right held + wheel", "Adjust inspection flight speed";
    Release => "Escape", "Release navigation focus and stop momentum";
}

impl NavigationGesture {
    /// Optical zoom takes priority over speed adjustment; outside the image the caller retains scrolling.
    pub const fn wheel(zoom: bool, looking: bool) -> Self {
        if zoom {
            Self::Zoom
        } else if looking {
            Self::Speed
        } else {
            Self::Dolly
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wheel_modifier_priority_and_help_cover_every_wheel_operation() {
        for zoom in [false, true] {
            for looking in [false, true] {
                let kind = NavigationGesture::wheel(zoom, looking);
                assert_eq!(
                    kind,
                    if zoom {
                        NavigationGesture::Zoom
                    } else if looking {
                        NavigationGesture::Speed
                    } else {
                        NavigationGesture::Dolly
                    }
                );
                assert!(NavigationGesture::ALL.contains(&kind));
                assert!(!kind.chord().is_empty() && !kind.description().is_empty());
            }
        }
    }
}
