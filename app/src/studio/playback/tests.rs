use super::*;
use recording_studio::{
    MediaTime, Project,
    preview::{PreviewQuality, PreviewRequest as VideoRequest},
};
use std::sync::Arc;

#[test]
fn pcm_history_retires_on_real_source_changes_but_not_view_or_forward_time() {
    let preview = VideoRequest {
        quality: PreviewQuality::Balanced,
        cadence: None,
        epoch: [1, 2, 3],
        project: Arc::new(Project::new("audio")),
        source_base: "original".into(),
        at: MediaTime::ZERO,
        camera_view: None,
    };
    let mut request = PreviewRequest {
        preview,
        duration: MediaTime(1_000_000),
        playing: true,
        ceiling: None,
        speed: Default::default(),
    };
    let key = PlaybackKey::new(&request, 48_000).unwrap();
    request.preview.at = MediaTime(123_000);
    request.preview.quality = PreviewQuality::Fast;
    assert!(key.matches(&request, 48_000));
    assert!(!key.matches(&request, 44_100));
    request.preview.source_base = "relocated".into();
    assert!(!key.matches(&request, 48_000));
    request.preview.source_base = "original".into();
    request.preview.project = Arc::new((*request.preview.project).clone());
    assert!(!key.matches(&request, 48_000));
    let key = PlaybackKey::new(&request, 48_000).unwrap();
    request.speed = recording_studio::playback::monitoring::MonitoringSpeed::Eight;
    assert!(!key.matches(&request, 48_000));
    request.speed = Default::default();
    request.preview.epoch[2] += 1;
    assert!(!key.matches(&request, 48_000));
}
