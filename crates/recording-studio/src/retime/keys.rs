//! Speed keys remain ordered and source-anchored while ordinary edit commands own undo and redo.

use super::*;

impl SpeedRamp {
    /// An exact source timestamp identifies one key without snapping to an unrelated neighbour.
    pub fn key_at(&self, at: MediaTime) -> Option<&SpeedKey> {
        self.keys
            .binary_search_by_key(&at, |key| key.at)
            .ok()
            .map(|index| &self.keys[index])
    }

    /// Inserting a key preserves an existing incoming easing choice when only its multiplier changes.
    pub fn keyed(&self, mut key: SpeedKey) -> Result<Self, StudioError> {
        let mut next = self.clone();
        match next.keys.binary_search_by_key(&key.at, |key| key.at) {
            Ok(index) => {
                key.easing = next.keys[index].easing;
                next.keys[index] = key;
            }
            Err(index) => {
                crate::error::limit("speed ramp keys", next.keys.len() + 1, MAX_SPEED_KEYS)?;
                next.keys.insert(index, key);
            }
        }
        next.validate(PlaybackRate::default())?;
        Ok(next)
    }

    /// Removing the final key clears the optional ramp and restores the clip's overall rate.
    pub fn without_key(&self, at: MediaTime) -> Result<Option<Self>, StudioError> {
        let index = self
            .keys
            .binary_search_by_key(&at, |key| key.at)
            .map_err(|_| StudioError::Invalid("select an exact speed key before deleting it"))?;
        let mut next = self.clone();
        next.keys.remove(index);
        if next.keys.is_empty() {
            return Ok(None);
        }
        next.validate(PlaybackRate::default())?;
        Ok(Some(next))
    }

    /// Incoming easing changes the continuous speed segment arriving at an exact source key.
    pub fn with_key_easing(&self, at: MediaTime, easing: Easing) -> Result<Self, StudioError> {
        let index = self
            .keys
            .binary_search_by_key(&at, |key| key.at)
            .map_err(|_| {
                StudioError::Invalid("select an exact speed key before changing easing")
            })?;
        let mut next = self.clone();
        next.keys[index].easing = easing;
        next.validate(PlaybackRate::default())?;
        Ok(next)
    }
}
