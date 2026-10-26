//! Calling-thread work counters for deterministic face-comparison regressions.

use std::cell::Cell;

thread_local! {
    static WORK: Cell<[usize; 2]> = const { Cell::new([0; 2]) };
}

/// Records executed fallback face cells and light-channel comparisons.
pub(in crate::stream) fn record(cells: usize, channels: usize) {
    WORK.set([WORK.get()[0] + cells, WORK.get()[1] + channels]);
}

/// Measures fallback cell visits and channel comparisons made by this synchronous operation.
pub(in crate::stream) fn measure<T>(operation: impl FnOnce() -> T) -> (T, [usize; 2]) {
    let before = WORK.get();
    let result = operation();
    let after = WORK.get();
    (
        result,
        std::array::from_fn(|index| after[index] - before[index]),
    )
}
