use super::*;
use recording_studio::{
    MediaTime, Project,
    camera::{CameraPose, CameraView},
};

#[test]
fn forward_time_preserves_worker_identity_but_same_time_camera_edits_replace_it() {
    let mut request = PreviewRequest {
        quality: Default::default(),
        cadence: None,
        epoch: [1, 2, 3],
        project: Arc::new(Project::new("preview")),
        source_base: "takes".into(),
        at: MediaTime(1_000_000),
        camera_view: None,
    };
    let identity = RequestIdentity::new(&request);
    request.at = MediaTime(2_000_000);
    assert!(identity.matches(&request));
    request.project = Arc::new((*request.project).clone());
    assert!(!identity.matches(&request));
    assert!(identity.same_generation(&request));
    let identity = RequestIdentity::new(&request);
    request.camera_view = Some(CameraView {
        pose: CameraPose::default(),
        revision: 1,
    });
    assert!(!identity.matches(&request));
    assert!(identity.same_generation(&request));
    let identity = RequestIdentity::new(&request);
    request.camera_view.as_mut().unwrap().revision += 1;
    assert!(!identity.matches(&request));
    assert!(identity.same_generation(&request));
    request.epoch[2] += 1;
    assert!(!identity.same_generation(&request));
}

#[test]
fn preview_quality_invalidates_old_extents_and_high_fps_uses_display_sized_credit() {
    use recording_studio::{FrameRate, preview::PreviewQuality};
    let mut request = request();
    Arc::make_mut(&mut request.project).export.fps = FrameRate {
        numerator: 240,
        denominator: 1,
    };
    let exported = request.frame_rate();
    assert_eq!(exported.numerator, 240);
    request.cadence = Some(request.quality.frame_rate(exported));
    assert_eq!(request.frame_rate().numerator, 60);
    let identity = RequestIdentity::new(&request);
    request.quality = PreviewQuality::Fast;
    request.cadence = Some(request.quality.frame_rate(exported));
    assert!(!identity.same_generation(&request));
    assert!(!identity.matches(&request));
    let displayed = PreviewStatus {
        ready: true,
        epoch: Some(request.epoch),
        cadence: request.cadence,
        ..Default::default()
    };
    assert_eq!(
        displayed.playback_ceiling(request.epoch, exported),
        request.frame_rate().frame_time(2).ok()
    );
    assert!(
        displayed
            .playback_ceiling(request.epoch, exported)
            .unwrap()
            .0
            > 60_000
    );
    assert_eq!(request.project.export.fps, exported);
    let ntsc = FrameRate {
        numerator: 30_000,
        denominator: 1001,
    };
    assert_eq!(PreviewQuality::Fast.frame_rate(ntsc), ntsc);
}

#[test]
fn full_preview_budget_does_not_cap_4k_240_fps_export_extent_or_clock() {
    use recording_studio::{
        FrameRate,
        preview::{PreviewQuality, fit_size},
    };
    let mut export = request();
    let project = Arc::make_mut(&mut export.project);
    project.export.size = [3840, 2160];
    project.export.fps = FrameRate {
        numerator: 240,
        denominator: 1,
    };
    export.quality = PreviewQuality::Full;
    export.cadence = None;
    export.at = export.project.export.fps.frame_time(239).unwrap();
    assert_eq!(export.frame_time().unwrap(), export.at);
    assert_eq!(export.frame_rate(), export.project.export.fps);
    assert_eq!(
        fit_size(export.project.export.size, export.project.export.size).unwrap(),
        [3840, 2160]
    );
    let (preview, maximum) = export
        .clone()
        .for_interactive(UI_STUDIO_PREVIEW_MAX_SIZE, true);
    assert_eq!(maximum, UI_STUDIO_PREVIEW_MAX_SIZE);
    assert_eq!(
        preview.frame_rate(),
        FrameRate {
            numerator: 60,
            denominator: 1
        }
    );
    let size = fit_size(preview.project.export.size, maximum).unwrap();
    assert!(
        size.into_iter()
            .zip(UI_STUDIO_PREVIEW_MAX_SIZE)
            .all(|(side, maximum)| side <= maximum)
    );
    assert!(preview.frame_time().unwrap() < export.frame_time().unwrap());
    assert_eq!(export.project.export.size, [3840, 2160]);
    assert_eq!(export.project.export.fps.numerator, 240);
    assert_eq!(export.cadence, None);
}

#[test]
fn paused_high_fps_steps_keep_the_exact_output_grid_at_every_preview_quality() {
    use recording_studio::{FrameRate, preview::PreviewQuality};
    let mut request = request();
    Arc::make_mut(&mut request.project).export.fps = FrameRate {
        numerator: 240,
        denominator: 1,
    };
    for quality in [
        PreviewQuality::Fast,
        PreviewQuality::Balanced,
        PreviewQuality::Full,
    ] {
        request.quality = quality;
        for frame in 1..=3 {
            request.at = request.project.export.fps.frame_time(frame).unwrap();
            let (playing, maximum) = request
                .clone()
                .for_interactive(UI_STUDIO_PREVIEW_MAX_SIZE, true);
            let (paused, paused_maximum) =
                playing.for_interactive(UI_STUDIO_PREVIEW_MAX_SIZE, false);
            assert_eq!(paused.cadence, None);
            assert_eq!(paused.frame_time().unwrap(), request.at);
            assert_eq!(paused.frame_rate(), request.project.export.fps);
            assert_eq!(paused_maximum, maximum);
            assert_eq!(maximum, quality.maximum_size(UI_STUDIO_PREVIEW_MAX_SIZE));
        }
    }
}

/// A keyed camera gives the pure scheduling fixture a finite duration without opening media files.
fn request() -> PreviewRequest {
    let mut project = Project::new("preview");
    project.shots.push(
        recording_studio::camera::CameraShot::from_preview(
            recording_studio::Id(1),
            recording_studio::Span::new(MediaTime::ZERO, MediaTime(1_000_000)).unwrap(),
            recording_studio::camera::CameraRigKind::Free,
            CameraPose::default(),
            None,
        )
        .unwrap(),
    );
    PreviewRequest {
        quality: Default::default(),
        cadence: None,
        epoch: [1, 2, 3],
        project: Arc::new(project),
        source_base: "takes".into(),
        at: MediaTime::ZERO,
        camera_view: None,
    }
}

#[test]
fn repeated_camera_refreshes_keep_playback_credit_and_actual_seek_withdraws_it() {
    let mut request = request();
    let mut displayed = PreviewStatus {
        ready: true,
        epoch: Some(request.epoch),
        at: MediaTime::ZERO,
        ..Default::default()
    };
    let ceiling = displayed.playback_ceiling(request.epoch, request.project.export.fps);
    for revision in 0..100 {
        request.camera_view = Some(CameraView {
            pose: CameraPose::default(),
            revision,
        });
        displayed = refresh_status(Some(&displayed), request.epoch, false);
        assert!(displayed.ready);
        assert!(!displayed.loading);
        assert_eq!(
            displayed.playback_ceiling(request.epoch, request.project.export.fps),
            ceiling
        );
    }
    let pending = refresh_status(Some(&displayed), request.epoch, true);
    assert!(!pending.ready);
    assert!(pending.loading);
    assert!(
        pending
            .playback_ceiling(request.epoch, request.project.export.fps)
            .is_none()
    );
    request.epoch[2] += 1;
    let pending = refresh_status(Some(&displayed), request.epoch, false);
    assert!(!pending.ready);
    assert!(pending.loading);
}

#[test]
fn paused_and_seeking_refreshes_retain_an_image_only_within_the_same_document() {
    let displayed = PreviewStatus {
        ready: true,
        epoch: Some([4, 5, 6]),
        at: MediaTime(2_000_000),
        ..Default::default()
    };
    for epoch in [[4, 5, 6], [4, 5, 7], [4, 6, 6]] {
        let pending = refresh_status(Some(&displayed), epoch, true);
        assert!(!pending.ready);
        assert!(pending.loading);
        assert_eq!(pending.epoch, displayed.epoch);
        assert_eq!(pending.at, displayed.at);
    }
    let other = refresh_status(Some(&displayed), [5, 0, 0], true);
    assert!(!other.ready);
    assert!(other.loading);
    assert!(other.epoch.is_none());
    assert_eq!(other.at, MediaTime::ZERO);
}

#[test]
fn equivalent_font_allocations_retain_the_owner_but_real_font_changes_invalidate() {
    let font = ui_engine::test_support::fixture_font();
    let replacement = Arc::new((*font).clone());
    assert!(!Arc::ptr_eq(&font, &replacement));
    let (retained, changed) = retained_font(Some(&font), replacement);
    assert!(!changed);
    assert!(Arc::ptr_eq(&font, &retained));
    let replacement = Arc::new(
        (*font)
            .clone()
            .with_rendering(assets::FontRendering::NativeSdf),
    );
    let (_, changed) = retained_font(Some(&font), replacement);
    assert!(changed);
}

#[test]
fn preparation_runs_exactly_one_frame_ahead_only_after_the_current_frame_is_admitted() {
    let mut request = request();
    let fps = request.project.export.fps;
    let mut displayed = PreviewStatus {
        ready: true,
        epoch: Some(request.epoch),
        at: MediaTime::ZERO,
        ..Default::default()
    };
    assert_eq!(
        prepare_time(&request, Some(&displayed), true, true).unwrap(),
        fps.frame_time(1).unwrap()
    );
    assert_eq!(
        prepare_time(&request, Some(&displayed), true, false).unwrap(),
        MediaTime::ZERO
    );
    assert_eq!(
        prepare_time(&request, Some(&displayed), false, true).unwrap(),
        MediaTime::ZERO
    );
    request.at = fps.frame_time(3).unwrap();
    assert_eq!(
        prepare_time(&request, Some(&displayed), true, true).unwrap(),
        request.at
    );
    for frame in 0..fps.frames_covering(request.project.duration()).unwrap() {
        request.at = fps.frame_time(frame).unwrap();
        displayed.at = request.at;
        let next = prepare_time(&request, Some(&displayed), true, true).unwrap();
        assert!(next < request.project.duration());
        let expected = fps.frame_time(frame + 1).unwrap();
        assert_eq!(
            next,
            if expected < request.project.duration() {
                expected
            } else {
                request.at
            }
        );
    }
}

#[test]
fn prepared_future_images_wait_until_due_and_cannot_replace_a_paused_view() {
    let request = request();
    let next = request.project.export.fps.frame_time(1).unwrap();
    assert_eq!(delivery(next, MediaTime::ZERO, true), Delivery::Hold);
    assert_eq!(delivery(next, next, true), Delivery::Show);
    assert_eq!(delivery(next, MediaTime::ZERO, false), Delivery::Discard);
    let state = NativePreview {
        displayed: Some(PreviewStatus {
            ready: true,
            epoch: Some(request.epoch),
            at: MediaTime::ZERO,
            ..Default::default()
        }),
        displayed_identity: Some(RequestIdentity::new(&request)),
        displayed_generation: Some(7),
        ..Default::default()
    };
    assert!(state.has_frame(&request, MediaTime::ZERO, 7));
    assert!(!state.has_frame(&request, MediaTime::ZERO, 8));
    let mut edited = request.clone();
    edited.camera_view = Some(CameraView {
        pose: CameraPose::default(),
        revision: 1,
    });
    assert!(!state.has_frame(&edited, MediaTime::ZERO, 7));
}

#[test]
fn late_coalesced_images_cannot_reverse_continuous_playback() {
    assert!(!follows_displayed(MediaTime(10), Some(MediaTime(20)), true));
    assert!(follows_displayed(MediaTime(20), Some(MediaTime(20)), true));
    assert!(follows_displayed(MediaTime(30), Some(MediaTime(20)), true));
    assert!(follows_displayed(MediaTime(10), None, true));
    assert!(follows_displayed(MediaTime(10), Some(MediaTime(20)), false));
}
