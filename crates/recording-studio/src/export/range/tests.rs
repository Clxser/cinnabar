use super::*;
use crate::{
    Id, PlaybackRate, Project,
    model::{Clip, ExportSettings, Framing, MediaKind, MediaSource, Transition, TransitionKind},
};

#[test]
fn full_output_retains_legacy_frame_count_and_integer_grid_timestamps() {
    let duration = MediaTime(1_234_567);
    for fps in [
        FrameRate {
            numerator: 60,
            denominator: 1,
        },
        FrameRate {
            numerator: 30_000,
            denominator: 1001,
        },
        FrameRate {
            numerator: 239,
            denominator: 1,
        },
    ] {
        let range = FrameRange::new(fps, duration, None).unwrap();
        assert_eq!(range.frames(), fps.frames_covering(duration).unwrap());
        for index in 0..range.frames() {
            assert_eq!(range.time(index).unwrap(), fps.frame_time(index).unwrap());
        }
    }
}

#[test]
fn custom_output_uses_global_rational_frame_indices_instead_of_rounded_offsets() {
    let fps = FrameRate {
        numerator: 30_000,
        denominator: 1001,
    };
    let span = Span::new(fps.frame_time(1).unwrap(), fps.frame_time(4).unwrap()).unwrap();
    let range = FrameRange::new(fps, MediaTime(1_000_000), Some(span)).unwrap();
    assert_eq!(range.frames(), 3);
    assert_eq!(range.time(0).unwrap(), span.start);
    assert_eq!(range.time(2).unwrap(), fps.frame_time(3).unwrap());
    assert_ne!(
        range.time(2).unwrap(),
        span.start.checked_add(fps.frame_time(2).unwrap()).unwrap()
    );
    assert!(range.time(3).is_err());
    assert!(
        FrameRange::new(
            fps,
            MediaTime(1_000_000),
            Some(Span::new(MediaTime(1), MediaTime(2)).unwrap())
        )
        .is_err()
    );
}

#[test]
fn pcm_origin_and_end_share_the_rational_frame_grid_and_never_extend_past_out() {
    let fps = FrameRate {
        numerator: 239,
        denominator: 1,
    };
    let span = Span::new(fps.frame_time(49).unwrap(), fps.frame_time(98).unwrap()).unwrap();
    let range = FrameRange::new(fps, MediaTime(1_000_000), Some(span)).unwrap();
    let rate = 48_000;
    let [origin, end, samples] = range.audio_bounds(rate).unwrap();
    assert_eq!(origin, (49 * u64::from(rate)).div_ceil(239));
    assert_eq!(samples, (range.frames() * u64::from(rate)).div_ceil(239));
    assert!(end <= origin + samples);
    assert!(origin + samples - end <= 1);
    assert!(range.audio_bounds(0).is_err());
}

#[test]
fn changed_cadence_admits_only_intervals_containing_an_exported_frame() {
    let fps = FrameRate {
        numerator: 24,
        denominator: 1,
    };
    let range = FrameRange::new(
        fps,
        MediaTime(1_000_000),
        Some(Span::new(MediaTime(33_333), MediaTime(100_000)).unwrap()),
    )
    .unwrap();
    let frames: Vec<_> = (0..range.frames())
        .map(|index| range.time(index).unwrap())
        .collect();
    assert_eq!(frames, [MediaTime(41_666), MediaTime(83_333)]);
    for (start, end, intersects) in [
        (0, 33_333, false),
        (33_333, 40_000, false),
        (41_666, 41_667, true),
        (41_667, 83_333, false),
        (83_333, 83_334, true),
        (83_334, 100_000, false),
        (100_000, 110_000, false),
    ] {
        let span = Span::new(MediaTime(start), MediaTime(end)).unwrap();
        assert_eq!(range.visual_intersects(span).unwrap(), intersects);
        assert_eq!(intersects, frames.iter().any(|at| span.contains(*at)));
    }
}

#[test]
fn pcm_admission_excludes_the_unsampled_prefix_but_keeps_sound_between_pictures() {
    let range = FrameRange::new(
        FrameRate {
            numerator: 24,
            denominator: 1,
        },
        MediaTime(1_000_000),
        Some(Span::new(MediaTime(33_333), MediaTime(100_000)).unwrap()),
    )
    .unwrap();
    let rate = crate::export::AUDIO_SAMPLE_RATE;
    for (start, end, audible) in [
        (33_333, 40_000, false),
        (41_666, 41_667, true),
        (44_001, 44_002, false),
        (45_000, 50_000, true),
        (90_000, 100_000, true),
        (100_000, 110_000, false),
    ] {
        let span = Span::new(MediaTime(start), MediaTime(end)).unwrap();
        assert_eq!(range.audio_window(span, rate).unwrap().is_some(), audible);
    }
    let between = Span::new(MediaTime(45_000), MediaTime(50_000)).unwrap();
    assert!(!range.visual_intersects(between).unwrap());
    assert_eq!(
        range.audio_window(between, rate).unwrap(),
        Some([
            crate::audio::sample_frame(between.start, rate).unwrap(),
            crate::audio::sample_frame(between.end, rate).unwrap(),
        ])
    );
}

#[test]
fn older_settings_default_to_full_output_and_stale_ranges_do_not_prevent_authoring() {
    let old: ExportSettings = serde_json::from_str(
        r#"{"size":[1280,720],"fps":{"numerator":120,"denominator":1},"codec":"h264"}"#,
    )
    .unwrap();
    assert!(old.range.is_none());
    assert_eq!(old.fps.numerator, 120);
    assert!(!serde_json::to_string(&old).unwrap().contains("\"range\""));
    let mut project = Project::new("stale range");
    project.export.range = Some(Span::new(MediaTime(500_000), MediaTime(1_000_000)).unwrap());
    project.validate().unwrap();
    assert!(FrameRange::new(project.export.fps, project.duration(), project.export.range).is_err());
}

/// Two clips retain original source clocks on either side of one centred dissolve.
fn dissolved() -> Project {
    let mut project = Project::new("transition handles");
    project.sources.push(MediaSource {
        id: Id(1),
        name: "source".into(),
        path: "unused.mp4".into(),
        kind: MediaKind::Video,
        duration: MediaTime(2_000_000),
        size: [16, 16],
        frame_rate: None,
        sha256: None,
    });
    for (id, at) in [(2, 0), (3, 1_000_000)] {
        project.clips.push(Clip {
            id: Id(id),
            source: Id(1),
            source_range: Span::new(MediaTime::ZERO, MediaTime(1_000_000)).unwrap(),
            at: MediaTime(at),
            rate: PlaybackRate::default(),
            speed_ramp: None,
            track: 0,
            framing: Framing::default(),
            enabled: true,
        });
    }
    project.transitions.push(Transition {
        id: Id(4),
        from: Id(2),
        to: Id(3),
        duration: MediaTime(200_000),
        kind: TransitionKind::Crossfade,
    });
    project
}

#[test]
fn range_source_admission_retains_both_crossfade_handles() {
    let project = dissolved();
    let coverage = visual_coverage(&project).unwrap();
    let handles = project.transitions[0].span_at(project.clips[1].at).unwrap();
    assert_eq!(coverage[&Id(2)].end, handles.end);
    assert_eq!(coverage[&Id(3)].start, handles.start);
    assert!(coverage[&Id(2)].contains(MediaTime(1_050_000)));
    assert!(coverage[&Id(3)].contains(MediaTime(950_000)));
}
