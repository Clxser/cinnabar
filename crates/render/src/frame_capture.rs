//! Sampling boundary for world recordings that leave the live screen HUD unchanged.

use bevy::prelude::SystemSet;

/// Runs after world post-processing, hands, projected text and camera overlays, before screen UI.
/// Consumers must copy the current view here; its main texture may contain HUD pixels afterward.
#[derive(Debug, Clone, Hash, Eq, PartialEq, SystemSet)]
pub struct WorldFrameCapture;
