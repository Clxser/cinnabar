//! Criterion timings for the same synthetic scenes as the CSV diagnostic.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion};
use world::{
    LightBlockSample, LightChannel, LightReadAccess, LightSolveOutput, LightSolverScratch,
};

use super::{
    DEFAULT_SECTIONS,
    fixtures::{Fixture, Prior, Scene},
    solve,
};

/// Checks light values and sky provenance before a case enters the timed loop.
fn check_output(fixture: &Fixture, actual: &LightSolveOutput, expected: &LightSolveOutput) {
    for position in fixture.positions() {
        for channel in [LightChannel::Block, LightChannel::Sky] {
            assert_eq!(
                actual.light_at(position, channel),
                expected.light_at(position, channel)
            );
        }
        assert_eq!(
            actual.has_direct_sky_provenance(0, position),
            expected.has_direct_sky_provenance(0, position),
        );
    }
}

/// Measures full and incremental solves, including output destruction but excluding setup.
pub(super) fn main() {
    let mut criterion = Criterion::default().configure_from_args();
    for full in [false, true] {
        let mut group = criterion.benchmark_group(if full {
            "world/light_work_full"
        } else {
            "world/light_work_incremental"
        });
        for sections in DEFAULT_SECTIONS {
            for scene in Scene::ALL {
                for retained in [false, true] {
                    if (scene == Scene::Removal && !retained) || (scene == Scene::Solid && retained)
                    {
                        continue;
                    }
                    let mut fixture = Fixture::new(scene, sections);
                    let mut scratch = LightSolverScratch::default();
                    let original = solve(
                        &fixture,
                        &Prior {
                            output: None,
                            fixture: &fixture,
                        },
                        &mut scratch,
                        false,
                    );
                    if scene == Scene::Removal {
                        fixture.samples.fill(LightBlockSample::KnownAir);
                    }
                    let prior = Prior {
                        output: retained.then_some(&original),
                        fixture: &fixture,
                    };
                    let expected = solve(&fixture, &prior, &mut scratch, false);
                    let output = solve(&fixture, &prior, &mut scratch, full);
                    check_output(&fixture, &output, &expected);
                    if retained && scene != Scene::Removal {
                        check_output(&fixture, &output, &original);
                    }
                    if scene == Scene::Solid || scene == Scene::Removal {
                        assert!(fixture.positions().all(|position| {
                            output.light_at(position, LightChannel::Block) == 0
                                && output.light_at(position, LightChannel::Sky) == 0
                        }));
                    }
                    drop(output);
                    drop(expected);
                    let state = if retained { "retained" } else { "fresh" };
                    let name = format!("synthetic_{}_{state}", scene.name());
                    group.bench_function(BenchmarkId::new(name, sections), |b| {
                        b.iter(|| drop(black_box(solve(&fixture, &prior, &mut scratch, full))));
                    });
                }
            }
        }
        group.finish();
    }
    criterion.final_summary();
}
