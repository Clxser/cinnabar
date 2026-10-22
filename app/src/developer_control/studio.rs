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
            "health": studio.capture.health,
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

#[cfg(test)]
mod tests {
    use super::*;
    use recording_studio::capture::CaptureHealth;

    #[test]
    fn capture_diagnostics_keep_cpu_latency_bytes_and_stalls_distinct() {
        let health = CaptureHealth {
            actor_prepare_micros: 101,
            sample_micros: 203,
            readback_micros: 307,
            pending_readbacks: 2,
            readback_bytes: 4096,
            encoder_frames: 3,
            encoder_bytes: 8192,
            encoder_stalls: 5,
            readback_stalls: 7,
        };
        let studio = Snapshot {
            capture: recording_studio::capture::CaptureStatus {
                health,
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(
            snapshot(&studio)["capture"]["health"],
            json!({
                "actor_prepare_micros": health.actor_prepare_micros,
                "sample_micros": health.sample_micros,
                "readback_micros": health.readback_micros,
                "pending_readbacks": health.pending_readbacks,
                "readback_bytes": health.readback_bytes,
                "encoder_frames": health.encoder_frames,
                "encoder_bytes": health.encoder_bytes,
                "encoder_stalls": health.encoder_stalls,
                "readback_stalls": health.readback_stalls,
            })
        );
    }
}
