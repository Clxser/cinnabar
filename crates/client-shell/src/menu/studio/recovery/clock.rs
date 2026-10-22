//! A bounded caller-clock interval cannot be postponed indefinitely by continuous edits.

use recording_studio::{MediaTime, storage::AUTOSAVE_INTERVAL_MICROS};

#[derive(Default)]
pub(super) struct Clock {
    document: u64,
    elapsed: u64,
    revision: Option<u64>,
}

impl Clock {
    /// A new document receives a fresh interval and completion identity.
    pub fn reset(&mut self, document: u64) {
        *self = Self {
            document,
            ..Default::default()
        };
    }
    /// An explicit retry admits the current revision on the next available worker turn.
    pub fn retry(&mut self) {
        self.elapsed = AUTOSAVE_INTERVAL_MICROS;
        self.revision = None;
    }
    /// Every submission, including failed IO, starts the next bounded interval.
    pub fn submitted(&mut self) {
        self.elapsed = 0;
    }
    /// Only a completion from the current document can acknowledge a revision.
    pub fn saved(&mut self, document: u64, revision: u64) {
        if document == self.document {
            self.revision = Some(revision);
        }
    }
    /// Invalid deltas cannot create a busy loop or skip the finite autosave interval.
    pub fn advance(&mut self, document: u64, revision: u64, dirty: bool, elapsed: f64) -> bool {
        if document != self.document {
            self.reset(document);
        }
        if !dirty {
            self.elapsed = 0;
            return false;
        }
        if let Ok(elapsed) = MediaTime::from_seconds(elapsed) {
            self.elapsed = self
                .elapsed
                .saturating_add(elapsed.0)
                .min(AUTOSAVE_INTERVAL_MICROS);
        }
        self.elapsed >= AUTOSAVE_INTERVAL_MICROS && self.revision != Some(revision)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn continuous_edits_do_not_starve_autosave_and_old_completions_do_not_mark_new_revisions() {
        let mut clock = Clock::default();
        assert!(!clock.advance(1, 1, true, 10.0));
        assert!(!clock.advance(1, 2, true, 10.0));
        assert!(clock.advance(1, 3, true, 10.0));
        clock.submitted();
        clock.saved(1, 3);
        assert!(!clock.advance(1, 3, true, 30.0));
        assert!(clock.advance(1, 4, true, 0.0));
        assert!(!clock.advance(2, 1, true, 0.0));
        clock.saved(1, 99);
        assert!(clock.advance(2, 1, true, 30.0));
        assert!(!clock.advance(2, 1, false, f64::INFINITY));
    }
}
