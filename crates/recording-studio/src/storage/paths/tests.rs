use super::*;
use crate::{
    Id, MediaTime,
    camera::{CameraPose, SavedCameraView},
    edit::Editor,
    model::{MediaKind, MediaSource},
    storage::{RecoveryRequest, RecoveryValue, StorageRequest, StorageValue},
};
use std::{fs, sync::Arc};

/// Real project-file IO is sufficient to exercise path portability; scene decoding is independent.
fn project(path: PathBuf) -> Project {
    let mut project = Project::new("Portable views");
    project.sources.push(MediaSource {
        id: Id(1),
        name: "Take".into(),
        path,
        kind: MediaKind::Replay,
        duration: MediaTime(2_000_000),
        size: [1280, 720],
        frame_rate: None,
        sha256: None,
    });
    project.camera_views.push(
        SavedCameraView::new(
            Id(2),
            "Gate".into(),
            &project.sources[0],
            CameraPose {
                position: [1.25, 70.5, -9.75],
                yaw: 33.5,
                pitch: -15.25,
                roll: 8.75,
                fov: 51.25,
            },
        )
        .unwrap(),
    );
    project
}

/// Uses the exact synchronous worker dispatch for both serialization and load-time resolution.
fn save_open(path: &Path, project: &Project, source_base: &Path) -> Editor {
    super::super::execute(StorageRequest::SaveProject {
        path: path.into(),
        project: Arc::new(project.clone()),
        revision: 1,
        source_base: source_base.into(),
    })
    .unwrap();
    open(path)
}

/// Opening through worker dispatch includes the active-editor absolute-path conversion.
fn open(path: &Path) -> Editor {
    let StorageValue::LoadedProject { editor, .. } =
        super::super::execute(StorageRequest::LoadProject { path: path.into() }).unwrap()
    else {
        panic!("loaded project");
    };
    editor
}

/// A recovery copy resolves paths against the document's source base, never the recovery directory.
fn recover(root: &Path, project: &Project, source_base: &Path) -> Editor {
    let mut store = super::super::recovery::Store::default();
    let RecoveryValue::Opened { id } = store
        .execute(RecoveryRequest::Begin { root: root.into() })
        .unwrap()
    else {
        panic!("opened recovery slot");
    };
    store
        .execute(RecoveryRequest::Save {
            root: root.into(),
            id: id.clone(),
            document: 1,
            revision: 1,
            project: Arc::new(project.clone()),
            original_file: None,
            source_base: source_base.into(),
        })
        .unwrap();
    let RecoveryValue::Restored { editor, .. } = store
        .execute(RecoveryRequest::Restore {
            root: root.into(),
            id,
        })
        .unwrap()
    else {
        panic!("restored project");
    };
    editor
}

#[test]
fn unhashed_view_survives_real_save_and_open_when_portable_paths_become_relative() {
    let root = tempfile::tempdir().unwrap();
    let take = root.path().join("take.cinrec");
    fs::write(&take, b"synthetic external take").unwrap();
    let project = project(take.clone());
    let before = project.clone();
    let path = root.path().join("project.cinstudio");
    let opened = save_open(&path, &project, root.path());
    let portable = super::super::files::load(&path).unwrap();
    assert_eq!(portable.sources[0].path, Path::new("take.cinrec"));
    assert_eq!(
        portable.camera_views[0].source_identity.path,
        Path::new("take.cinrec")
    );
    assert_eq!(opened.project().sources[0].path, take);
    assert_eq!(
        opened.project().camera_views[0]
            .recall(opened.project(), Id(1))
            .unwrap(),
        project.camera_views[0].pose
    );
    assert_eq!(project, before);
}

#[test]
fn save_as_resolves_relative_bindings_from_the_old_base_and_save_new_preserves_them() {
    let root = tempfile::tempdir().unwrap();
    let original_base = root.path().join("old");
    fs::create_dir_all(original_base.join("media")).unwrap();
    fs::write(
        original_base.join("media/take.cinrec"),
        b"synthetic external take",
    )
    .unwrap();
    let original = project("media/take.cinrec".into());
    let first = original_base.join("project.cinstudio");
    save_open(&first, &original, &original_base);
    let serialized_old = super::super::files::load(&first).unwrap();
    let save_as = root.path().join("copy.cinstudio");
    let opened = save_open(&save_as, &serialized_old, &original_base);
    let portable = super::super::files::load(&save_as).unwrap();
    assert_eq!(portable.sources[0].path, Path::new("old/media/take.cinrec"));
    assert_eq!(
        portable.camera_views[0].source_identity.path,
        portable.sources[0].path
    );
    assert_eq!(
        opened.project().sources[0].path,
        original_base.join("media/take.cinrec")
    );
    assert!(
        opened.project().camera_views[0]
            .recall(opened.project(), Id(1))
            .is_ok()
    );
    assert_eq!(
        serialized_old.camera_views[0].source_identity.path,
        Path::new("media/take.cinrec")
    );
    let StorageValue::SavedProject { path, .. } =
        super::super::execute(StorageRequest::SaveNewProject {
            directory: root.path().join("new"),
            project: Arc::clone(opened.project()),
            revision: 2,
            source_base: root.path().into(),
        })
        .unwrap()
    else {
        panic!("saved new project");
    };
    let reopened = open(&path);
    assert_eq!(
        reopened.project().camera_views[0]
            .recall(reopened.project(), Id(1))
            .unwrap(),
        original.camera_views[0].pose
    );
}

#[test]
fn recovery_and_a_subsequent_explicit_save_preserve_relative_source_bound_views() {
    let root = tempfile::tempdir().unwrap();
    let source_base = root.path().join("document");
    fs::create_dir_all(source_base.join("media")).unwrap();
    fs::write(
        source_base.join("media/take.cinrec"),
        b"synthetic external take",
    )
    .unwrap();
    let project = project("media/take.cinrec".into());
    let restored = recover(&root.path().join("autosave"), &project, &source_base);
    assert_eq!(
        restored.project().sources[0].path,
        source_base.join("media/take.cinrec")
    );
    assert_eq!(
        restored.project().camera_views[0].source_identity.path,
        restored.project().sources[0].path
    );
    assert_eq!(
        restored.project().camera_views[0]
            .recall(restored.project(), Id(1))
            .unwrap(),
        project.camera_views[0].pose
    );
    let opened = save_open(
        &root.path().join("recovered.cinstudio"),
        restored.project(),
        &source_base,
    );
    assert_eq!(
        opened.project().camera_views[0]
            .recall(opened.project(), Id(1))
            .unwrap(),
        project.camera_views[0].pose
    );
}

#[test]
fn stale_relinked_view_bindings_stay_stale_across_save_as_open_and_recovery() {
    let root = tempfile::tempdir().unwrap();
    let source_base = root.path().join("document");
    fs::create_dir_all(source_base.join("media")).unwrap();
    for name in ["old.cinrec", "replacement.cinrec"] {
        fs::write(
            source_base.join("media").join(name),
            b"synthetic external take",
        )
        .unwrap();
    }
    let mut project = project("media/old.cinrec".into());
    project.sources[0].path = "media/replacement.cinrec".into();
    assert!(project.camera_views[0].recall(&project, Id(1)).is_err());
    let opened = save_open(
        &root.path().join("relocated.cinstudio"),
        &project,
        &source_base,
    );
    let restored = recover(&root.path().join("autosave"), &project, &source_base);
    for copy in [opened.project(), restored.project()] {
        assert_eq!(
            copy.sources[0].path,
            source_base.join("media/replacement.cinrec")
        );
        assert_eq!(
            copy.camera_views[0].source_identity.path,
            source_base.join("media/old.cinrec")
        );
        assert!(copy.camera_views[0].recall(copy, Id(1)).is_err());
    }
}
