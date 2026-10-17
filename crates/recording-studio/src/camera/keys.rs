//! Path key edits preserve ordered source time, authored easing and at least one usable pose.

use super::*;

impl CameraShot {
    /// Key times are offsets within the shot; non-path rigs expose an empty slice.
    pub fn path_keys(&self) -> &[CameraKey] {
        match &self.mode {
            CameraMode::Path { keys, .. } => keys,
            _ => &[],
        }
    }

    /// An exact key includes the legal final boundary even though shot rendering is half-open.
    pub fn key_at(&self, at: MediaTime) -> Option<&CameraKey> {
        if at < self.span.start || at > self.span.end {
            return None;
        }
        let offset = at.saturating_sub(self.span.start);
        let keys = self.path_keys();
        keys.binary_search_by_key(&offset, |key| key.at)
            .ok()
            .map(|index| &keys[index])
    }

    /// Removing an exact key retains the neighbouring rail and cannot remove its last pose.
    pub fn without_key(&self, at: MediaTime) -> Result<Self, StudioError> {
        let offset = self
            .key_at(at)
            .ok_or(StudioError::Invalid(
                "move to an exact camera key before deleting it",
            ))?
            .at;
        let mut next = self.clone();
        let CameraMode::Path { keys, .. } = &mut next.mode else {
            unreachable!()
        };
        if keys.len() == 1 {
            return Err(StudioError::Invalid(
                "a camera path must keep at least one key",
            ));
        }
        keys.retain(|key| key.at != offset);
        next.validate()?;
        Ok(next)
    }

    /// Easing belongs to the transition arriving at this exact key and survives pose replacement.
    pub fn with_key_easing(&self, at: MediaTime, easing: Easing) -> Result<Self, StudioError> {
        let offset = self
            .key_at(at)
            .ok_or(StudioError::Invalid(
                "move to an exact camera key before changing easing",
            ))?
            .at;
        let mut next = self.clone();
        let CameraMode::Path { keys, .. } = &mut next.mode else {
            unreachable!()
        };
        let key = keys
            .iter_mut()
            .find(|key| key.at == offset)
            .expect("admitted key");
        key.easing = easing;
        next.validate()?;
        Ok(next)
    }

    /// Moving a key preserves its pose and easing, rejects collisions and keeps the rail ordered.
    pub fn with_key_time(
        &self,
        at: MediaTime,
        destination: MediaTime,
    ) -> Result<Self, StudioError> {
        let offset = self
            .key_at(at)
            .ok_or(StudioError::Invalid(
                "move to an exact camera key before moving it",
            ))?
            .at;
        if destination < self.span.start || destination > self.span.end {
            return Err(StudioError::Invalid("camera key time is outside its shot"));
        }
        let destination = destination.saturating_sub(self.span.start);
        let mut next = self.clone();
        let CameraMode::Path { keys, .. } = &mut next.mode else {
            unreachable!()
        };
        if destination != offset && keys.iter().any(|key| key.at == destination) {
            return Err(StudioError::Invalid(
                "another camera key already occupies that time",
            ));
        }
        let key = keys
            .iter_mut()
            .find(|key| key.at == offset)
            .expect("admitted key");
        key.at = destination;
        keys.sort_unstable_by_key(|key| key.at);
        next.validate()?;
        Ok(next)
    }

    /// Smooth rails use time-weighted monotone tangents; linear rails retain the same easing choices.
    pub fn with_path_smoothing(&self, smooth: bool) -> Result<Self, StudioError> {
        let mut next = self.clone();
        let CameraMode::Path {
            smooth_position, ..
        } = &mut next.mode
        else {
            return Err(StudioError::Invalid(
                "select a path camera before changing smoothing",
            ));
        };
        *smooth_position = smooth;
        next.validate()?;
        Ok(next)
    }
}
