//! Bounded retries for recoverable GPU resource failures, driven by preparation calls.

const MAX_FAILURES: u8 = 7;

/// Retries a one-shot failure immediately, then backs off persistent allocation pressure.
#[derive(Default)]
pub(crate) struct AllocationRetry {
    failures: u8,
    remaining: u8,
}

impl AllocationRetry {
    /// Consumes one preparation opportunity without reading or blocking on the clock.
    pub(crate) fn ready(&mut self) -> bool {
        if self.remaining == 0 {
            true
        } else {
            self.remaining -= 1;
            false
        }
    }

    /// Backs off repeated failures to at most one attempt per 64 preparation calls.
    pub(crate) fn failed(&mut self) {
        self.failures = self.failures.saturating_add(1).min(MAX_FAILURES);
        self.remaining = (1_u8 << (self.failures - 1)) - 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_shot_fault_retries_immediately_and_persistent_faults_back_off() {
        let mut retry = AllocationRetry::default();
        assert!(retry.ready());
        retry.failed();
        assert!(retry.ready());
        retry.failed();
        assert!(!retry.ready());
        assert!(retry.ready());
        for _ in 0..32 {
            retry.failed();
            for _ in 0..retry.remaining {
                assert!(!retry.ready());
            }
            assert!(retry.ready());
        }
        assert_eq!(retry.failures, MAX_FAILURES);
        retry.failed();
        assert_eq!(retry.remaining, (1_u8 << (MAX_FAILURES - 1)) - 1);
    }
}
