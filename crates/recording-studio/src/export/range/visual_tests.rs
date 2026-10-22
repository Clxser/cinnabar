use super::*;

#[test]
fn exact_rounded_output_timestamps_remain_inside_a_one_microsecond_window() {
    for fps in [
        FrameRate::default(),
        FrameRate {
            numerator: 30_000,
            denominator: 1001,
        },
    ] {
        let range = FrameRange::new(fps, MediaTime(1_000_000), None).unwrap();
        for frame in 0..range.frames() {
            let at = range.time(frame).unwrap();
            let span = Span::new(at, at.checked_add(MediaTime(1)).unwrap()).unwrap();
            assert_eq!(range.visual_window(span).unwrap(), Some(span));
            assert!(range.visual_intersects(span).unwrap());
        }
    }
}

#[test]
fn an_unsampled_interval_between_frames_needs_no_subject_coverage() {
    let fps = FrameRate::default();
    let range = FrameRange::new(fps, MediaTime(1_000_000), None).unwrap();
    let span = Span::new(MediaTime(1), fps.frame_time(1).unwrap()).unwrap();
    assert_eq!(range.visual_window(span).unwrap(), None);
    assert!(!range.visual_intersects(span).unwrap());
}
