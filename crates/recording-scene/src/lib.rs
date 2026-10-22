//! Portable native scene journals for independent Recording Studio cameras.
//!
//! Callers lend immutable prepared meshes. Worker code freezes assets and validates decoded state;
//! this owner contains no live session, window, Bevy schedule or GPU device.

pub mod actors;
mod binary;
pub mod capture;
pub mod environment;
pub mod journal;
pub mod palette;
pub mod playback;
pub mod scene;
pub mod targets;
pub mod terrain;
pub mod visuals;

pub const TERRAIN_TRACK_SCHEMA: &str = "cinnabar.terrain/2";
