//! Source-owned verification admits only target intervals from the same immutable native take.

use recording_studio::{
    camera::coverage::{TargetCoverage, TargetCoverageRequest},
    capture::CameraRecording,
    export::Cancellation,
};

/// Scans target changes on a cancellable worker; present malformed tracks never become empty coverage.
pub fn verify_coverage(
    request: &TargetCoverageRequest,
    cancellation: &Cancellation,
) -> Result<Option<TargetCoverage>, String> {
    cancellation.check()?;
    request
        .source_range
        .validate()
        .map_err(|error| error.to_string())?;
    let camera = CameraRecording::open(&request.directory).map_err(|error| error.to_string())?;
    cancellation.check()?;
    let primary = camera.info();
    let mut targets = super::TargetRecording::open_optional(&request.directory)
        .map_err(|error| error.to_string())?
        .ok_or("This native take has no recorded subject track to verify.")?;
    cancellation.check()?;
    let mut identity = targets.info().clone();
    identity.scene_schema.clone_from(&primary.scene_schema);
    if &identity != primary {
        return Err("Camera target coverage belongs to a different recorded take.".into());
    }
    let mut request = request.clone();
    request.source_range.end = request.source_range.end.min(camera.duration());
    if request.source_range.start >= request.source_range.end {
        return Ok(None);
    }
    targets
        .continuous_coverage(&request, &|| cancellation.cancelled())
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests;
