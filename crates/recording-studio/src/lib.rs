//! Recording projects, reversible timeline edits and event-driven cinematic decisions.
//!
//! Media stays in immutable external recordings. This crate owns small project descriptions;
//! callers supply time, world samples and rendered frames. Storage runs behind bounded workers.

pub mod archive;
pub mod automation;
pub mod camera;
pub mod capture;
pub mod edit;
pub mod effect;
pub mod error;
pub mod model;
pub mod object_store;
pub mod playback;
pub mod storage;
pub mod time;
pub mod video;
pub mod wav;

pub use error::StudioError;
pub use model::{Id, Project};
pub use time::{FrameRate, MediaTime, PlaybackRate, Span};
