use super::*;
use crate::{
    edit::{Command, Editor},
    model::MediaSource,
};

/// Metadata-only replay identity needs no recording files or platform camera state.
fn project() -> Project {
    let mut project = Project::new("Saved camera views");
    project.sources.push(MediaSource {
        id: Id(1),
        name: "Take".into(),
        path: "take.cinrec".into(),
        kind: MediaKind::Replay,
        duration: MediaTime(2_000_000),
        size: [1280, 720],
        frame_rate: None,
        sha256: Some([7; 32]),
    });
    project
}

/// All six authored pose components deliberately differ from their defaults.
fn view(project: &Project) -> SavedCameraView {
    SavedCameraView::new(
        Id(2),
        "Courtyard wide".into(),
        &project.sources[0],
        CameraPose {
            position: [12.25, -4.5, 80.75],
            yaw: 137.25,
            pitch: -22.5,
            roll: 13.75,
            fov: 47.5,
        },
    )
    .unwrap()
}

#[test]
fn legacy_projects_default_to_no_views_and_saved_names_and_all_pose_values_round_trip() {
    let mut project = project();
    let mut json = serde_json::to_value(&project).unwrap();
    json.as_object_mut().unwrap().remove("camera_views");
    assert!(
        Project::from_json(&serde_json::to_vec(&json).unwrap())
            .unwrap()
            .camera_views
            .is_empty()
    );
    let saved = view(&project);
    project.camera_views.push(saved.clone());
    let opened = Project::from_json(&project.to_json().unwrap()).unwrap();
    assert_eq!(opened.camera_views, vec![saved.clone()]);
    assert_eq!(saved.recall(&opened, Id(1)).unwrap(), saved.pose);
    assert_eq!(opened.next_id().unwrap(), Id(3));
}

#[test]
fn add_rename_delete_and_source_deletion_are_complete_undoable_transactions() {
    let project = project();
    let saved = view(&project);
    let mut editor = Editor::new(project).unwrap();
    editor
        .apply(
            "Save view",
            &[Command::AddCameraView {
                view: saved.clone(),
            }],
        )
        .unwrap();
    let mut renamed = saved.clone();
    renamed.name = "Gate close-up".into();
    editor
        .apply(
            "Rename view",
            &[Command::ReplaceCameraView {
                view: renamed.clone(),
            }],
        )
        .unwrap();
    editor
        .apply("Delete view", &[Command::DeleteCameraView { id: saved.id }])
        .unwrap();
    assert!(editor.project().camera_views.is_empty());
    editor.undo().unwrap();
    assert_eq!(editor.project().camera_views, vec![renamed]);
    editor.undo().unwrap();
    assert_eq!(editor.project().camera_views, vec![saved.clone()]);
    editor.redo().unwrap();
    assert_eq!(editor.project().camera_views[0].name, "Gate close-up");
    let before = editor.project().clone();
    editor
        .apply("Delete source", &[Command::Delete { id: Id(1) }])
        .unwrap();
    assert!(editor.project().camera_views.is_empty() && editor.project().sources.is_empty());
    assert!(saved.recall(editor.project(), Id(1)).is_err());
    editor.undo().unwrap();
    assert_eq!(editor.project(), &before);
}

#[test]
fn source_relinking_is_conservative_and_rebinding_is_explicit_and_undoable() {
    let mut project = project();
    let saved = view(&project);
    project.camera_views.push(saved.clone());
    let mut relocated = project.sources[0].clone();
    relocated.path = "relocated/take.cinrec".into();
    assert!(saved.matches_source(&relocated));
    let mut replacement = relocated.clone();
    replacement.sha256 = Some([8; 32]);
    let mut editor = Editor::new(project).unwrap();
    editor
        .apply(
            "Relink take",
            &[Command::ReplaceSource {
                source: replacement.clone(),
            }],
        )
        .unwrap();
    assert!(saved.recall(editor.project(), Id(1)).is_err());
    let rebound = saved.rebound(&replacement).unwrap();
    assert_eq!(rebound.pose, saved.pose);
    editor
        .apply(
            "Bind to current take",
            &[Command::ReplaceCameraView {
                view: rebound.clone(),
            }],
        )
        .unwrap();
    assert_eq!(rebound.recall(editor.project(), Id(1)).unwrap(), saved.pose);
    assert!(rebound.recall(editor.project(), Id(99)).is_err());
    editor.undo().unwrap();
    assert!(
        editor.project().camera_views[0]
            .recall(editor.project(), Id(1))
            .is_err()
    );
    editor.undo().unwrap();
    assert_eq!(saved.recall(editor.project(), Id(1)).unwrap(), saved.pose);
    let mut unhashed = replacement;
    unhashed.sha256 = None;
    let fallback = saved.rebound(&unhashed).unwrap();
    assert!(fallback.matches_source(&unhashed));
    unhashed.path = "another-take.cinrec".into();
    assert!(!fallback.matches_source(&unhashed));
}

#[test]
fn malformed_names_poses_and_removed_or_nonreplay_sources_never_enter_saved_projects() {
    let mut project = project();
    let saved = view(&project);
    for name in [
        "".to_owned(),
        " \t ".into(),
        "line\nbreak".into(),
        "a".repeat(MAX_SAVED_VIEW_NAME_BYTES + 1),
    ] {
        let mut invalid = saved.clone();
        invalid.name = name;
        assert!(invalid.validate(&project.sources[0]).is_err());
    }
    let mut invalid = saved.clone();
    invalid.pose.position[0] = f64::NAN;
    assert!(invalid.validate(&project.sources[0]).is_err());
    project.camera_views.push(saved);
    project.sources[0].kind = MediaKind::Video;
    assert!(project.validate().is_err());
    project.sources.clear();
    assert!(project.validate().is_err());
}
