//! Export admission samples the exact output clock instead of requiring unseen source-time continuity.

use std::path::Path;

use recording_studio::{MediaTime, camera::CameraTargetRef, export::Cancellation};

/// Opens one admitted target cursor, seeks once, then streams nondecreasing required source timestamps.
/// A legacy runtime reference binds to its first sampled spawn; subsequent reuse cannot retarget the shot.
pub fn verify_samples(
    directory: &Path,
    source_times: impl IntoIterator<Item = Result<MediaTime, String>>,
    target: CameraTargetRef,
    require_eye: bool,
    cancellation: &Cancellation,
) -> Result<Option<CameraTargetRef>, String> {
    let (recording, duration) = super::coverage::open(directory, cancellation)?;
    recording.verify_samples(source_times, target, require_eye, duration, cancellation)
}
