//! Undoable effect authoring operates on private metadata copies at exact timeline positions.

use super::*;

impl EffectClip {
    /// Inserts an exact-playhead key; existing keys preserve their authored easing.
    pub fn keyed(&self, at: MediaTime, value: f64) -> Result<Self, StudioError> {
        if at < self.span.start || at > self.span.end || !value.is_finite() {
            return Err(StudioError::Invalid(
                "effect key is outside its clip or has an invalid amount",
            ));
        }
        let mut next = self.clone();
        let at = at.saturating_sub(self.span.start);
        let value = self.kind.constrain(value);
        match next.amount.keys.binary_search_by_key(&at, |key| key.at) {
            Ok(index) => next.amount.keys[index].value = value,
            Err(index) => {
                limit(
                    "effect curve keys",
                    next.amount.keys.len().saturating_add(1),
                    MAX_CURVE_KEYS,
                )?;
                next.amount.keys.insert(
                    index,
                    CurveKey {
                        at,
                        value,
                        easing: Easing::Smooth,
                    },
                );
            }
        }
        next.amount.validate(next.span.duration())?;
        Ok(next)
    }

    /// The inspector adjusts the sampled amount, leaving all other keys editable and retained.
    pub fn adjusted(&self, at: MediaTime, steps: f64) -> Result<Self, StudioError> {
        if !steps.is_finite() {
            return Err(StudioError::Invalid("effect adjustment must be finite"));
        }
        self.keyed(
            at,
            self.amount.sample(at.saturating_sub(self.span.start)) + steps * self.kind.step(),
        )
    }

    /// Removes an exact key only; a valid effect always retains at least one key.
    pub fn without_key(&self, at: MediaTime) -> Result<Self, StudioError> {
        if at < self.span.start || at > self.span.end {
            return Err(StudioError::Invalid("effect key is outside its clip"));
        }
        if self.amount.keys.len() <= 1 {
            return Err(StudioError::Invalid("an effect must retain one amount key"));
        }
        let offset = at.saturating_sub(self.span.start);
        let index = self
            .amount
            .keys
            .binary_search_by_key(&offset, |key| key.at)
            .map_err(|_| StudioError::Invalid("there is no effect key at this playhead"))?;
        let mut next = self.clone();
        next.amount.keys.remove(index);
        Ok(next)
    }

    /// The selected easing belongs to the key at the playhead; no nearby key is silently changed.
    pub fn eased(&self, at: MediaTime, easing: Easing) -> Result<Self, StudioError> {
        let mut next = self.keyed(at, self.amount.sample(at.saturating_sub(self.span.start)))?;
        let offset = at.saturating_sub(self.span.start);
        let index = next
            .amount
            .keys
            .binary_search_by_key(&offset, |key| key.at)
            .expect("key inserted above");
        next.amount.keys[index].easing = easing;
        Ok(next)
    }
}
