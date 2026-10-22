use super::fixture::{bookmark, event, project, recording};
use super::*;
use crate::{
    MediaTime,
    model::{AudioRole, MarkerKind},
    wav::WavWriter,
};
use std::{collections::BTreeMap, fs, path::PathBuf};

/// Complete float PCM is synthetic; the resulting finalized file can also model a crash before header patching.
fn audio(parent: &Path, frames: usize, rate: u32) {
    let mut writer =
        WavWriter::create_rf64(&parent.join(crate::capture::AUDIO_FILE), 2, rate).unwrap();
    writer.write(std::iter::repeat_n(0.25, frames * 2)).unwrap();
    writer.finish().unwrap();
}

/// Original journal, metadata and audio bytes are compared recursively after recovery publishes its separate copy.
fn originals(parent: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    /// Recursively collects only the synthetic fixture's files for byte-for-byte preservation checks.
    fn visit(root: &Path, parent: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(parent).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, files);
            } else {
                files.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    visit(parent, parent, &mut files);
    files
}

#[test]
fn short_audio_is_real_duration_unlinked_game_audio_and_bookmarks_retain_source_provenance() {
    let root = tempfile::tempdir().unwrap();
    let mut recording = recording(root.path());
    event(
        &mut recording,
        MediaTime(5_000),
        1,
        MediaTime(5_000),
        bookmark("Keep this"),
    );
    let prefix = crate::FrameRate {
        numerator: 30_000,
        denominator: 1001,
    }
    .frame_time(3)
    .unwrap();
    let task = recording.finish(prefix).unwrap();
    task.wait();
    audio(task.path(), 480, 48_000);
    let before = originals(task.path());
    let recovered = recover(
        project(task.path(), prefix),
        task.path(),
        true,
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(recovered.editor.duration(), prefix);
    assert!(recovered.editor.dirty());
    let project = recovered.editor.project();
    assert!(project.export.include_audio);
    assert_eq!(project.audio.len(), 1);
    let clip = &project.audio[0];
    assert_eq!(clip.role, AudioRole::Game);
    assert_eq!(clip.linked_clip, None);
    assert_eq!(clip.source_range.end, MediaTime(10_000));
    let source = project
        .sources
        .iter()
        .find(|source| source.id == clip.source)
        .unwrap();
    assert_eq!(source.duration, MediaTime(10_000));
    assert_ne!(source.path, task.path().join(crate::capture::AUDIO_FILE));
    assert!(source.path.is_file());
    assert_eq!(project.markers.len(), 1);
    let marker = &project.markers[0];
    assert_eq!(
        (marker.kind, marker.at),
        (MarkerKind::Manual, MediaTime(5_000))
    );
    let origin = marker.origin.as_ref().unwrap();
    assert_eq!(
        (
            origin.source,
            origin.clip_id,
            origin.event_sequence,
            origin.source_time
        ),
        (crate::Id(1), Some(crate::Id(2)), 1, Some(MediaTime(5_000)))
    );
    assert!(recovered.notice.contains("unlinked Game audio"));
    assert!(recovered.notice.contains("gap resets were not persisted"));
    for (path, bytes) in before {
        assert_eq!(fs::read(task.path().join(path)).unwrap(), bytes);
    }
}

#[test]
fn audio_covering_the_exact_scene_interval_remains_linked_without_extending_the_source() {
    let root = tempfile::tempdir().unwrap();
    let recording = recording(root.path());
    let prefix = MediaTime(100_000);
    let task = recording.finish(prefix).unwrap();
    task.wait();
    audio(task.path(), 200, 1_000);
    let recovered = recover(
        project(task.path(), prefix),
        task.path(),
        true,
        &Cancellation::default(),
    )
    .unwrap();
    let project = recovered.editor.project();
    assert_eq!(project.audio[0].linked_clip, Some(crate::Id(2)));
    assert_eq!(project.audio[0].source_range.end, prefix);
    assert_eq!(
        project
            .sources
            .iter()
            .find(|source| source.kind == MediaKind::Audio)
            .unwrap()
            .duration,
        prefix
    );
}

#[test]
fn missing_or_malformed_audio_is_reported_and_does_not_hide_durable_bookmarks() {
    for malformed in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let mut recording = recording(root.path());
        event(
            &mut recording,
            MediaTime(1_000),
            1,
            MediaTime(1_000),
            bookmark("Durable"),
        );
        let prefix = MediaTime(10_000);
        let task = recording.finish(prefix).unwrap();
        task.wait();
        if malformed {
            fs::write(
                task.path().join(crate::capture::AUDIO_FILE),
                b"invalid RF64",
            )
            .unwrap();
        }
        let before = originals(task.path());
        let recovered = recover(
            project(task.path(), prefix),
            task.path(),
            true,
            &Cancellation::default(),
        )
        .unwrap();
        assert!(recovered.editor.project().audio.is_empty());
        assert!(!recovered.editor.project().export.include_audio);
        assert_eq!(recovered.editor.project().markers.len(), 1);
        assert!(recovered.notice.contains("Game audio was not recovered:"));
        assert_eq!(originals(task.path()), before);
    }
}

#[test]
fn malformed_event_payload_keeps_valid_audio_but_never_admits_partial_marker_lists() {
    let root = tempfile::tempdir().unwrap();
    let mut recording = recording(root.path());
    event(
        &mut recording,
        MediaTime(1_000),
        1,
        MediaTime(1_000),
        bookmark("Do not partially publish"),
    );
    recording
        .push(crate::archive::RecordInput {
            at: MediaTime(2_000),
            kind: crate::archive::RecordKind::Gameplay,
            payload: std::sync::Arc::from(&b"invalid JSON"[..]),
        })
        .unwrap();
    let prefix = MediaTime(10_000);
    let task = recording.finish(prefix).unwrap();
    task.wait();
    audio(task.path(), 480, 48_000);
    let recovered = recover(
        project(task.path(), prefix),
        task.path(),
        true,
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(recovered.editor.project().audio.len(), 1);
    assert!(recovered.editor.project().markers.is_empty());
    assert!(
        recovered
            .notice
            .contains("Event markers were not recovered:")
    );
}

#[test]
fn disabled_audio_and_cancelled_recovery_never_publish_a_sidecar_copy() {
    let root = tempfile::tempdir().unwrap();
    let recording = recording(root.path());
    let prefix = MediaTime(10_000);
    let task = recording.finish(prefix).unwrap();
    task.wait();
    audio(task.path(), 480, 48_000);
    let before = originals(task.path());
    let recovered = recover(
        project(task.path(), prefix),
        task.path(),
        false,
        &Cancellation::default(),
    )
    .unwrap();
    assert!(recovered.editor.project().audio.is_empty());
    assert!(recovered.notice.contains("Game audio was disabled"));
    assert_eq!(originals(task.path()), before);
    let cancellation = Cancellation::default();
    assert!(cancellation.cancel());
    assert!(
        recover(
            project(task.path(), prefix),
            task.path(),
            true,
            &cancellation
        )
        .is_err()
    );
    assert_eq!(originals(task.path()), before);
}

#[test]
fn edited_or_retimed_projects_cannot_receive_raw_source_time_companions() {
    let root = tempfile::tempdir().unwrap();
    let recording = recording(root.path());
    let prefix = MediaTime(10_000);
    let task = recording.finish(prefix).unwrap();
    task.wait();
    let mut project = project(task.path(), prefix);
    project.clips[0].at = MediaTime(5_000);
    assert!(recover(project, task.path(), false, &Cancellation::default()).is_err());
}
