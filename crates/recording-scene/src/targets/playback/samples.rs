//! Exact sample verification reuses raw target snapshots and interpolates only the selected subject.

use super::*;
use recording_studio::{camera::CameraTargetRef, export::Cancellation};

impl TargetRecording {
    /// Consumes its isolated cursor: one initial seek, then bounded lookahead through required source times.
    pub(crate) fn verify_samples(
        mut self,
        source_times: impl IntoIterator<Item = Result<MediaTime, String>>,
        target: CameraTargetRef,
        require_eye: bool,
        duration: MediaTime,
        cancellation: &Cancellation,
    ) -> Result<Option<CameraTargetRef>, String> {
        let mut previous = None;
        let mut lifetime = None;
        for at in source_times {
            cancellation.check()?;
            let at = at?;
            if previous.is_some_and(|previous| at < previous) {
                return Err("Camera verification source timestamps must be nondecreasing.".into());
            }
            if at >= duration {
                return Ok(None);
            }
            if previous.is_none() {
                self.restore_states(at, &|| cancellation.cancelled())
            } else {
                self.advance_states(at, &|| cancellation.cancelled())
            }
            .map_err(|error| error.to_string())?;
            let reference = lifetime.unwrap_or(target);
            let current = self
                .current
                .as_ref()
                .ok_or("Camera verification has no target baseline.")?;
            let Some(subject) = current
                .target(reference.runtime_id)
                .filter(|subject| subject.matches(reference))
            else {
                return Ok(None);
            };
            let pose = interpolation(current.at, self.next.as_ref(), at)
                .map_or(subject.pose, |(next, amount)| {
                    target_pose(subject, next, amount)
                });
            if require_eye && pose.eye_offset.is_none() {
                return Ok(None);
            }
            lifetime = Some(subject.reference());
            previous = Some(at);
        }
        cancellation.check()?;
        Ok(lifetime)
    }
}

#[cfg(test)]
mod tests;
