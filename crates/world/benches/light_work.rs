//! Synthetic solver measurements; these do not measure scheduler skips or live frame budgets.

#[cfg(not(feature = "runtime-profile"))]
#[path = "light_work/diagnostics.rs"]
mod diagnostics;
#[path = "light_work/fixtures.rs"]
mod fixtures;
#[cfg(feature = "runtime-profile")]
#[path = "light_work/runtime_profile.rs"]
mod runtime_profile;

use std::hint::black_box;

use fixtures::{Fixture, Prior};
use world::{
    LightSolveOutput, LightSolverScratch, solve_light_full_with_scratch, solve_light_with_scratch,
};

#[cfg(not(feature = "runtime-profile"))]
#[global_allocator]
static ALLOCATOR: test_alloc::CountingAllocator = test_alloc::CountingAllocator;

#[cfg(feature = "benchmark-mimalloc")]
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

const DEFAULT_SECTIONS: [usize; 3] = [1, 10, 24];

/// Solves one prepared synthetic scene, including its retained input and reusable scratch.
fn solve(
    fixture: &Fixture,
    prior: &Prior<'_>,
    scratch: &mut LightSolverScratch,
    full: bool,
) -> LightSolveOutput {
    let solve = if full {
        solve_light_full_with_scratch
    } else {
        solve_light_with_scratch
    };
    solve(
        black_box(fixture),
        black_box(prior),
        fixture.bounds,
        17,
        fixture.profile(),
        fixture.limits(),
        scratch,
    )
    .expect("valid synthetic lighting fixture")
}

/// Selects the allocation-counting diagnostic or the opt-in Criterion measurements.
fn main() {
    #[cfg(not(feature = "runtime-profile"))]
    diagnostics::main();
    #[cfg(feature = "runtime-profile")]
    runtime_profile::main();
}
