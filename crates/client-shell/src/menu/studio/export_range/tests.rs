use super::*;

/// A metadata-only project leaves range and export-request tests independent of devices and media files.
fn studio() -> Studio {
    let mut project = Project::new("range edit");
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
    project.clips.push(Clip {
        id: Id(2),
        source: Id(1),
        source_range: Span::new(MediaTime::ZERO, MediaTime(2_000_000)).unwrap(),
        at: MediaTime::ZERO,
        rate: PlaybackRate::default(),
        speed_ramp: None,
        track: 0,
        framing: Framing::default(),
        enabled: true,
    });
    let mut studio = Studio::new("unused".into());
    studio.install(Editor::new(project).unwrap(), None);
    studio
}

#[test]
fn typed_range_is_one_undoable_setting_edit_without_mutating_clip_timing_or_output_quality() {
    let mut studio = studio();
    let original = Arc::clone(studio.project().unwrap());
    studio.begin_export_range(ExportRangeField::In).unwrap();
    studio.range_text.set_text("0.333333");
    studio.apply_export_range().unwrap();
    let saved = studio.project().unwrap();
    assert_eq!(
        saved.export.range.unwrap().start,
        saved.export.fps.frame_time(20).unwrap()
    );
    assert_eq!(saved.clips, original.clips);
    assert_eq!(saved.export.size, original.export.size);
    assert_eq!(saved.export.fps, original.export.fps);
    assert_eq!(saved.export.quality, original.export.quality);
    studio.editor.as_mut().unwrap().undo().unwrap();
    assert!(studio.project().unwrap().export.range.is_none());
}

#[test]
fn invalid_and_stale_fields_leave_project_metadata_unchanged() {
    let mut studio = studio();
    studio.begin_export_range(ExportRangeField::Out).unwrap();
    let original = Arc::clone(studio.project().unwrap());
    for value in ["NaN", "inf", "-1", "0", "900000"] {
        studio.range_text.set_text(value);
        assert!(studio.apply_export_range().is_err());
        assert!(Arc::ptr_eq(studio.project().unwrap(), &original));
    }
    studio
        .editor
        .as_mut()
        .unwrap()
        .apply(
            "intervening edit",
            &[Command::Rename {
                name: "changed".into(),
            }],
        )
        .unwrap();
    studio.range_text.set_text("1");
    assert!(studio.apply_export_range().is_err());
    assert!(studio.project().unwrap().export.range.is_none());
}

#[test]
fn playhead_bounds_and_request_use_the_same_admitted_project_timestamps() {
    let mut studio = studio();
    studio
        .transport
        .seek(MediaTime(500_000), MediaTime(2_000_000));
    studio
        .export_range_at_playhead(ExportRangeField::In)
        .unwrap();
    studio
        .transport
        .seek(MediaTime(1_500_000), MediaTime(2_000_000));
    studio
        .export_range_at_playhead(ExportRangeField::Out)
        .unwrap();
    studio.queue_export().unwrap();
    let request = studio.export_request.as_ref().unwrap();
    let range = request.frame_range().unwrap();
    assert_eq!(range.time(0).unwrap(), MediaTime(500_000));
    assert_eq!(range.frames(), 60);
    assert_eq!(
        range.audio_bounds(48_000).unwrap(),
        [24_000, 72_000, 48_000]
    );
}

#[test]
fn queue_rejects_an_unapplied_field_and_full_project_clears_a_stale_saved_range() {
    let mut studio = studio();
    studio.begin_export_range(ExportRangeField::In).unwrap();
    assert!(studio.queue_export().is_err());
    studio.cancel_export_range();
    let mut settings = studio.project().unwrap().export.clone();
    settings.range = Some(Span::new(MediaTime(3_000_000), MediaTime(4_000_000)).unwrap());
    studio
        .editor
        .as_mut()
        .unwrap()
        .apply("stale range", &[Command::SetExport { settings }])
        .unwrap();
    assert!(studio.queue_export().is_err());
    studio.full_export_range().unwrap();
    studio.queue_export().unwrap();
    assert_eq!(
        studio
            .export_request
            .as_ref()
            .unwrap()
            .frame_range()
            .unwrap()
            .time(0)
            .unwrap(),
        MediaTime::ZERO
    );
}
