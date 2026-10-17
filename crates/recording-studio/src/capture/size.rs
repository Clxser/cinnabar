//! Recording limits share the output presets while preserving the source viewport's aspect.

use serde::{Deserialize, Serialize};

use crate::{settings::SIZE_PRESETS, video::frame_byte_len};

const _: () = assert!(SIZE_PRESETS.len() == 3);

/// A bounded video output limit; native scene tracks retain their original viewport coordinates.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum CaptureSize {
    #[default]
    Native,
    P720,
    P1080,
    P1440,
}

impl CaptureSize {
    /// Cycles through the same finite choices admitted by serialized recording preferences.
    pub const fn next(self) -> Self {
        match self {
            Self::Native => Self::P720,
            Self::P720 => Self::P1080,
            Self::P1080 => Self::P1440,
            Self::P1440 => Self::Native,
        }
    }

    /// Returns the shared landscape bounds, or no limit for an original-size readback.
    pub const fn bounds(self) -> Option<[u32; 2]> {
        match self {
            Self::Native => None,
            Self::P720 => Some(SIZE_PRESETS[0]),
            Self::P1080 => Some(SIZE_PRESETS[1]),
            Self::P1440 => Some(SIZE_PRESETS[2]),
        }
    }

    /// Labels limits rather than promising a fixed aspect or enlarging a smaller viewport.
    pub fn label(self) -> String {
        self.bounds()
            .map_or_else(|| "Native".into(), |size| format!("Up to {}p", size[1]))
    }

    /// Fits inside a landscape or portrait limit without upscaling; reduced dimensions are even.
    pub fn fit(self, source: [u32; 2]) -> Result<[u32; 2], String> {
        if source.iter().any(|&side| side < 2) {
            return Err("The recording window is minimized or too small.".into());
        }
        let mut size = source;
        if let Some(mut bounds) = self.bounds() {
            if source[1] > source[0] {
                bounds.swap(0, 1);
            }
            if source[0] > bounds[0] || source[1] > bounds[1] {
                let width_limited = u64::from(source[0]) * u64::from(bounds[1])
                    >= u64::from(source[1]) * u64::from(bounds[0]);
                let (limit, side) = if width_limited {
                    (bounds[0], source[0])
                } else {
                    (bounds[1], source[1])
                };
                size = source.map(|value| {
                    (u64::from(value) * u64::from(limit) / u64::from(side)) as u32 & !1
                });
                if size.iter().any(|&side| side < 2) {
                    return Err("The window's aspect is too narrow for this recording size.".into());
                }
            }
        }
        frame_byte_len(size)?;
        Ok(size)
    }
}
