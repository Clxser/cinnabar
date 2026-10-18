//! Independent lens keys retain their original easing phase across camera trims and splits.

use super::*;
use crate::effect::{Curve, CurveKey};

impl CameraShot {
    /// The optional lens track overrides every rig, including actor-driven cameras.
    pub fn fov_override(&self, at: MediaTime) -> Option<f64> {
        self.fov_curve.as_ref().map(|curve| {
            curve.sample(MediaTime(
                at.saturating_sub(self.span.start)
                    .0
                    .min(self.span.duration().0)
                    .saturating_add(self.fov_offset.0),
            ))
        })
    }

    /// Only keys in the retained shot interval can be selected or dragged; boundary support stays saved.
    pub fn fov_keys(&self) -> impl Iterator<Item = (MediaTime, &CurveKey)> {
        self.fov_curve
            .iter()
            .flat_map(|curve| &curve.keys)
            .filter_map(|key| {
                let relative = key.at.0.checked_sub(self.fov_offset.0)?;
                (relative <= self.span.duration().0)
                    .then(|| (MediaTime(self.span.start.0 + relative), key))
            })
    }

    /// Key lookup accepts an inclusive shot end, matching path-key authoring.
    pub fn fov_key_at(&self, at: MediaTime) -> Option<&CurveKey> {
        self.fov_keys()
            .find(|(time, _)| *time == at)
            .map(|(_, key)| key)
    }

    /// The first key retains the starting lens; subsequent value edits keep the key's incoming easing.
    pub fn with_fov_key(&self, at: MediaTime, value: f64) -> Result<Self, StudioError> {
        if at < self.span.start
            || at > self.span.end
            || !value.is_finite()
            || !CAMERA_FOV_RANGE.contains(&value)
        {
            return Err(StudioError::Invalid(
                "FOV key must be inside its shot with a valid lens value",
            ));
        }
        let mut next = self.clone();
        if next.fov_curve.is_none() {
            next.fov_offset = MediaTime::ZERO;
            next.fov_curve = Some(Curve::constant(
                self.parameter(CameraParameter::Fov, self.span.start)
                    .ok_or(StudioError::Invalid("camera has no field of view"))?,
            ));
        }
        let offset = at
            .saturating_sub(self.span.start)
            .checked_add(next.fov_offset)?;
        let keys = &mut next.fov_curve.as_mut().expect("lens curve installed").keys;
        match keys.binary_search_by_key(&offset, |key| key.at) {
            Ok(index) => keys[index].value = value,
            Err(index) => keys.insert(
                index,
                CurveKey {
                    at: offset,
                    value,
                    easing: Easing::Smooth,
                },
            ),
        }
        next.validate()?;
        Ok(next)
    }

    /// Removing the final lens key restores the original rig/path FOV without changing its motion.
    pub fn without_fov_key(&self, at: MediaTime) -> Result<Self, StudioError> {
        let offset = self
            .fov_key_at(at)
            .ok_or(StudioError::Invalid("select an exact FOV key first"))?
            .at;
        let mut next = self.clone();
        let curve = next.fov_curve.as_mut().expect("admitted lens key");
        curve.keys.retain(|key| key.at != offset);
        if curve.keys.is_empty() {
            next.fov_curve = None;
            next.fov_offset = MediaTime::ZERO;
        }
        next.validate()?;
        Ok(next)
    }

    /// Easing changes only the chosen lens transition, leaving position/orientation keys untouched.
    pub fn with_fov_easing(&self, at: MediaTime, easing: Easing) -> Result<Self, StudioError> {
        let offset = self
            .fov_key_at(at)
            .ok_or(StudioError::Invalid("select an exact FOV key first"))?
            .at;
        let mut next = self.clone();
        let key = next
            .fov_curve
            .as_mut()
            .expect("admitted lens key")
            .keys
            .iter_mut()
            .find(|key| key.at == offset)
            .expect("admitted lens key");
        key.easing = easing;
        Ok(next)
    }

    /// Moving a lens key cannot replace a neighbour or alter its value/easing.
    pub fn with_fov_key_time(
        &self,
        at: MediaTime,
        destination: MediaTime,
    ) -> Result<Self, StudioError> {
        let offset = self
            .fov_key_at(at)
            .ok_or(StudioError::Invalid("select an exact FOV key first"))?
            .at;
        if destination < self.span.start || destination > self.span.end {
            return Err(StudioError::Invalid("FOV key time is outside its shot"));
        }
        let wanted = destination
            .saturating_sub(self.span.start)
            .checked_add(self.fov_offset)?;
        let mut next = self.clone();
        let keys = &mut next.fov_curve.as_mut().expect("admitted lens key").keys;
        if offset != wanted && keys.iter().any(|key| key.at == wanted) {
            return Err(StudioError::Invalid(
                "another FOV key already occupies that time",
            ));
        }
        keys.iter_mut()
            .find(|key| key.at == offset)
            .expect("admitted lens key")
            .at = wanted;
        keys.sort_unstable_by_key(|key| key.at);
        next.validate()?;
        Ok(next)
    }
}

#[cfg(test)]
mod tests;
