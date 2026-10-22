use super::*;

#[test]
fn new_preferences_skip_video_but_legacy_take_metadata_keeps_its_video_contract() {
    let settings = CaptureSettings::default();
    assert_eq!(settings.mode, CaptureMode::ReplayOnly);
    let mut json = serde_json::to_value(settings).unwrap();
    json.as_object_mut().unwrap().remove("mode");
    let legacy: CaptureSettings = serde_json::from_value(json).unwrap();
    assert_eq!(legacy.mode, CaptureMode::ReplayAndVideo);
    assert!(legacy.mode.records_video());
    assert!(!settings.mode.records_video());
    assert_eq!(
        serde_json::from_str::<CaptureSettings>(&serde_json::to_string(&settings).unwrap())
            .unwrap(),
        settings
    );
}

#[test]
fn native_session_has_no_video_sink_and_preserves_interrupted_metadata_without_a_proxy() {
    let root = tempfile::tempdir().unwrap();
    let info = crate::archive::RecordingInfo {
        version: crate::archive::RECORDING_VERSION,
        name: "Native session fixture".into(),
        game_version: "fixture".into(),
        protocol_version: 1,
        scene_schema: String::new(),
        created_unix_ms: 1,
        viewport: [640, 360],
        local_actor: 1,
    };
    let settings = CaptureSettings::default();
    let mut session = CaptureSession::create(root.path(), info, settings).unwrap();
    assert!(session.sink().is_none());
    let directory = session.directory().to_path_buf();
    session.camera(MediaTime::ZERO, Default::default()).unwrap();
    let duration = settings.fps.frame_time(3).unwrap();
    let result = session.finish(duration, None, 0, 0);
    assert!(result.err().unwrap().contains("scene tracks"));
    assert!(!directory.join(VIDEO_FILE).exists());
    assert!(!CaptureManifest::exists(&directory));
    let mut camera = CameraRecording::open(&directory).unwrap();
    assert!(camera.restore(MediaTime::ZERO, || false).is_ok());
}
