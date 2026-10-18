//! Read-only creator diagnostics expose clocks and bounded metadata through the existing local endpoint.

use launcher::menu::studio::Snapshot;
use serde_json::{Value, json};

/// Reports admitted editor/capture state without traversing recorded geometry or opening media files.
pub(super) fn snapshot(studio: &Snapshot) -> Value {
    let project = studio.project.as_ref();
    json!({
        "library": studio.show_library,
        "loading": studio.loading,
        "playing": studio.playing,
        "position_us": studio.position.0,
        "duration_us": project.map(|project| project.duration().0),
        "selected": studio.selected,
        "source_count": project.map(|project| project.sources.len()),
        "shot_count": project.map(|project| project.shots.len()),
        "dirty": studio.dirty,
        "error": studio.error,
        "notice": studio.notice,
        "capture": {
            "phase": format!("{:?}", studio.capture.phase),
            "elapsed_us": studio.capture.elapsed.0,
            "frames": studio.capture.captured_frames,
            "missed_frames": studio.capture.missed_frames,
            "fps": studio.capture.fps,
            "audio": studio.capture.audio,
            "path": studio.capture.path,
            "message": studio.capture.message,
        },
        "preview": {
            "ready": studio.preview.ready,
            "loading": studio.preview.loading,
            "at_us": studio.preview.at.0,
            "epoch": studio.preview.epoch,
            "error": studio.preview.error,
            "notice": studio.preview.notice,
            "camera": studio.preview.camera.as_ref().map(|camera| json!({
                "interactive": camera.interactive,
                "source_time_us": camera.source_time.0,
                "pose": camera.render_pose,
                "target_count": camera.targets.len(),
                "local_actor": camera.local_actor,
            })),
        },
        "audio": {
            "engaged": studio.audio.engaged,
            "ready": studio.audio.ready,
            "buffering": studio.audio.buffering,
            "position_us": studio.audio.position.0,
            "buffered_until_us": studio.audio.buffered_until.0,
            "underruns": studio.audio.underruns,
            "error": studio.audio.error,
        },
        "camera_focus": studio.camera_workspace.focused,
        "inspection_view": studio.camera_workspace.view.map(|view| view.pose),
        "export": {
            "phase": format!("{:?}", studio.export.phase),
            "frames": studio.export.frames,
            "total_frames": studio.export.total_frames,
            "error": studio.export.error,
            "path": studio.export.path,
        },
    })
}
