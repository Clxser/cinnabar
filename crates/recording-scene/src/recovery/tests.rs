use super::*;

/// Recovery fixtures use original metadata and no installed game assets.
fn info() -> RecordingInfo {
    RecordingInfo {
        version: recording_studio::archive::RECORDING_VERSION,
        name: "Interrupted synthetic take".into(),
        game_version: "fixture".into(),
        protocol_version: 0,
        scene_schema: recording_studio::capture::POV_SCENE_SCHEMA.into(),
        created_unix_ms: 0,
        viewport: [1280, 720],
        local_actor: 1,
    }
}

#[test]
fn recovered_project_contains_only_verified_common_prefix_and_keeps_native_camera_access() {
    let settings = CaptureSettings::default();
    let duration = settings.fps.frame_time(7).unwrap();
    let mut visited = Vec::new();
    let path = Path::new("immutable-original.cinreplay");
    let editor = admitted_project(path, &info(), settings, duration, |at| {
        visited.push(at);
        Ok(())
    })
    .unwrap();
    assert_eq!(
        visited,
        (0..=7)
            .map(|frame| settings.fps.frame_time(frame).unwrap())
            .collect::<Vec<_>>()
    );
    assert!(editor.dirty());
    let project = editor.project();
    assert_eq!(project.duration(), duration);
    assert_eq!(project.sources[0].kind, MediaKind::Replay);
    assert_eq!(project.sources[0].path, path);
    assert_eq!(project.sources[0].duration, duration);
    assert_eq!(project.clips[0].source_range.end, duration);
    assert!(project.audio.is_empty());
    assert_eq!(project.export.fps, settings.fps);
}

#[test]
fn transient_missing_material_or_cancelled_middle_frame_cannot_be_reported_as_recovered() {
    let settings = CaptureSettings::default();
    let failure_at = settings.fps.frame_time(3).unwrap();
    for reason in ["missing hash-checked material asset", "Export cancelled."] {
        let mut visited = Vec::new();
        let result = admitted_project(
            Path::new("immutable-original.cinreplay"),
            &info(),
            settings,
            settings.fps.frame_time(7).unwrap(),
            |at| {
                visited.push(at);
                if at == failure_at {
                    Err(reason.into())
                } else {
                    Ok(())
                }
            },
        );
        assert_eq!(result.err().as_deref(), Some(reason));
        assert_eq!(visited.last(), Some(&failure_at));
        assert_eq!(visited.len(), 4);
    }
}

#[test]
fn empty_common_prefix_never_opens_payloads_or_invents_a_playable_frame() {
    let result = admitted_project(
        Path::new("immutable-original.cinreplay"),
        &info(),
        CaptureSettings::default(),
        MediaTime::ZERO,
        |_| panic!("empty prefix must not read a source frame"),
    );
    assert!(result.is_err());
}

#[test]
fn fractional_capture_cadence_is_preserved_and_exact_common_end_is_validated() {
    let settings = CaptureSettings {
        fps: recording_studio::FrameRate {
            numerator: 30_000,
            denominator: 1001,
        },
        ..Default::default()
    };
    let duration = MediaTime(85_000);
    let mut visited = Vec::new();
    let editor = admitted_project(
        Path::new("take.cinreplay"),
        &info(),
        settings,
        duration,
        |at| {
            visited.push(at);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(
        visited,
        vec![
            MediaTime::ZERO,
            MediaTime(33_366),
            MediaTime(66_733),
            duration
        ]
    );
    assert_eq!(editor.project().export.fps, settings.fps);
    assert_eq!(editor.duration(), duration);
}

#[test]
fn recovery_retains_original_scene_coordinates_but_prepares_even_export_dimensions() {
    let mut info = info();
    info.viewport = [1281, 721];
    let settings = CaptureSettings::default();
    let editor = admitted_project(
        Path::new("take.cinreplay"),
        &info,
        settings,
        settings.fps.frame_time(1).unwrap(),
        |_| Ok(()),
    )
    .unwrap();
    assert_eq!(editor.project().sources[0].size, info.viewport);
    assert_eq!(editor.project().export.size, [1280, 720]);
}
