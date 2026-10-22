use super::*;
use crate::{
    PlaybackRate,
    camera::TargetLifetime,
    effect::Easing,
    retime::{SpeedKey, SpeedRamp},
};

/// Small integer clocks exercise exact rounding at every reachable microsecond.
fn interval(start: u64, end: u64) -> Span {
    Span::new(MediaTime(start), MediaTime(end)).unwrap()
}

/// A source proof names the same spawn lifetime independently of a clip's placement.
fn proof(start: u64, end: u64) -> TargetCoverage {
    TargetCoverage {
        source_range: interval(start, end),
        target: CameraTargetRef {
            runtime_id: 1,
            lifetime: Some(TargetLifetime {
                unique_id: 2,
                spawn_revision: 3,
            }),
        },
    }
}

#[test]
fn retiming_preserves_the_last_covered_microsecond_without_sampling_past_despawn() {
    for rate in [
        PlaybackRate {
            numerator: 1,
            denominator: 2,
        },
        PlaybackRate::default(),
        PlaybackRate {
            numerator: 3,
            denominator: 2,
        },
    ] {
        let map = TimeMap::new(interval(20, 100), MediaTime(50), rate, None).unwrap();
        let coverage = proof(20, 61);
        let prefix = timeline_prefix(&map, map.span(), MediaTime(200), coverage)
            .unwrap()
            .unwrap();
        for at in map.span().start.0..map.span().end.0 {
            assert_eq!(
                prefix.contains(MediaTime(at)),
                coverage
                    .source_range
                    .contains(map.transition_time(MediaTime(at), MediaTime(200)).unwrap())
            );
        }
        assert_eq!(
            source_window(&map, prefix, MediaTime(200)).unwrap().start,
            MediaTime(20)
        );
    }
}

#[test]
fn speed_ramps_and_dissolve_handles_use_the_same_source_clock_as_rendering() {
    let ramp = SpeedRamp {
        keys: vec![
            SpeedKey {
                at: MediaTime(0),
                rate: PlaybackRate {
                    numerator: 1,
                    denominator: 2,
                },
                easing: Easing::Linear,
            },
            SpeedKey {
                at: MediaTime(200),
                rate: PlaybackRate {
                    numerator: 2,
                    denominator: 1,
                },
                easing: Easing::Smooth,
            },
        ],
    };
    let map = TimeMap::new(
        interval(30, 180),
        MediaTime(100),
        PlaybackRate::default(),
        Some(&ramp),
    )
    .unwrap();
    let timeline = interval(90, map.span().end.0 + 10);
    let coverage = proof(0, 120);
    let prefix = timeline_prefix(&map, timeline, MediaTime(250), coverage)
        .unwrap()
        .unwrap();
    for at in timeline.start.0..timeline.end.0 {
        assert_eq!(
            prefix.contains(MediaTime(at)),
            map.transition_time(MediaTime(at), MediaTime(250)).unwrap() < coverage.source_range.end
        );
    }
    assert!(source_window(&map, prefix, MediaTime(250)).unwrap().end <= coverage.source_range.end);
}

#[test]
fn gaps_before_the_shot_cannot_be_repaired_by_a_later_valid_interval() {
    let map = TimeMap::new(
        interval(0, 100),
        MediaTime(0),
        PlaybackRate::default(),
        None,
    )
    .unwrap();
    assert_eq!(
        timeline_prefix(&map, map.span(), MediaTime(100), proof(10, 90)).unwrap(),
        None
    );
    assert_eq!(
        timeline_prefix(&map, interval(90, 100), MediaTime(100), proof(0, 90)).unwrap(),
        None
    );
    assert_eq!(
        source_window(&map, interval(99, 100), MediaTime(100)).unwrap(),
        interval(99, 100)
    );
}
