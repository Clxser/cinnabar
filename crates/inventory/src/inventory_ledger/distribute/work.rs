//! Thread-local deterministic accounting for drag staging and projection work.

use std::cell::Cell;

thread_local! {
    static LEDGER_CLONES: Cell<usize> = const { Cell::new(0) };
    static REFOLDS: Cell<usize> = const { Cell::new(0) };
}

#[derive(Debug, Default)]
pub(crate) struct LedgerCloneProbe;

impl Clone for LedgerCloneProbe {
    /// Counts actual derived ledger clones, including clones in future drag helpers.
    fn clone(&self) -> Self {
        LEDGER_CLONES.with(|count| count.set(count.get() + 1));
        Self
    }
}

/// Counts a full prediction fold wherever it is triggered.
pub(in crate::inventory_ledger) fn note_refold() {
    REFOLDS.with(|count| count.set(count.get() + 1));
}

/// Takes this thread's ledger clone and full projection fold counts.
pub(in crate::inventory_ledger) fn take() -> (usize, usize) {
    (
        LEDGER_CLONES.with(|count| count.replace(0)),
        REFOLDS.with(|count| count.replace(0)),
    )
}
