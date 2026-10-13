//! Shared cancellation for bounded graphics work on CPU and GPU workers.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// A run's cancellation remains set for every worker, including late callbacks.
#[derive(Clone, Debug, Default)]
pub struct BenchmarkCancellation(Arc<AtomicBool>);

impl BenchmarkCancellation {
    /// Stops admission of further work without waiting for the GPU or a worker thread.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    /// Checks cancellation before submitting the next bounded unit of work.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_reaches_existing_workers_and_late_callbacks() {
        let run = BenchmarkCancellation::default();
        let worker = run.clone();
        assert!(!worker.is_cancelled());
        run.cancel();
        assert!(worker.is_cancelled());
        assert!(run.clone().is_cancelled());
        assert!(!BenchmarkCancellation::default().is_cancelled());
    }
}
