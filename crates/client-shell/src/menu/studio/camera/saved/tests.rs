use super::*;
use crate::menu::studio::camera::authoring_tests::inspected_project;
use recording_studio::camera::CameraPose;

/// Restores the metadata-only ready preview after a document edit, without a rendering worker.
fn ready(studio: &mut Studio) {
    let epoch = [
        studio.document,
        studio.editor.as_ref().unwrap().revision(),
        studio.transport.epoch(),
    ];
    let preview = &mut Arc::make_mut(&mut studio.view).preview;
    preview.ready = true;
    preview.epoch = Some(epoch);
    preview.camera.as_mut().unwrap().epoch = epoch;
}

#[test]
fn naming_is_transient_until_saved_and_recall_restores_exact_pose_without_dirtying_or_seeking() {
    let mut studio = inspected_project(false);
    let pose = CameraPose {
        position: [4.5, 70.25, -9.75],
        yaw: 81.25,
        pitch: -15.5,
        roll: 11.75,
        fov: 53.25,
    };
    studio.inspect_camera_pose(pose);
    let revision = studio.editor.as_ref().unwrap().revision();
    let dirty = studio.editor.as_ref().unwrap().dirty();
    studio.begin_camera_view_name(None).unwrap();
    studio.camera_view_text.set_text("Tower view");
    studio.publish_camera_view_name();
    assert_eq!(studio.editor.as_ref().unwrap().revision(), revision);
    assert_eq!(studio.editor.as_ref().unwrap().dirty(), dirty);
    studio.inspect_camera_pose(CameraPose::default());
    studio.camera_views_action(V::Save).unwrap();
    let saved = studio.project().unwrap().camera_views[0].clone();
    assert_eq!(saved.pose, pose);
    assert_eq!(saved.name, "Tower view");
    let revision = studio.editor.as_ref().unwrap().revision();
    studio.editor.as_mut().unwrap().mark_saved(revision);
    ready(&mut studio);
    let revision = studio.editor.as_ref().unwrap().revision();
    let epoch = studio.transport.epoch();
    studio.camera_views_action(V::Recall(saved.id)).unwrap();
    assert_eq!(studio.view.camera_workspace.view.unwrap().pose, pose);
    assert_eq!(studio.editor.as_ref().unwrap().revision(), revision);
    assert!(!studio.editor.as_ref().unwrap().dirty());
    assert_eq!(studio.transport.epoch(), epoch);
    let encoded = studio.project().unwrap().to_json().unwrap();
    studio.install(
        Editor::new(Project::from_json(&encoded).unwrap()).unwrap(),
        None,
    );
    assert_eq!(studio.project().unwrap().camera_views, vec![saved]);
    assert!(studio.view.camera_workspace.view.is_none());
}

#[test]
fn renamed_and_deleted_views_round_trip_through_undo_and_invalid_recall_preserves_inspection() {
    let mut studio = inspected_project(false);
    studio.begin_camera_view_name(None).unwrap();
    studio.camera_views_action(V::Save).unwrap();
    let id = studio.project().unwrap().camera_views[0].id;
    studio.begin_camera_view_name(Some(id)).unwrap();
    studio.camera_view_text.set_text("North bridge");
    studio.camera_views_action(V::Save).unwrap();
    assert_eq!(
        studio.project().unwrap().camera_views[0].name,
        "North bridge"
    );
    studio.camera_views_action(V::Delete(id)).unwrap();
    assert!(studio.project().unwrap().camera_views.is_empty());
    studio.editor.as_mut().unwrap().undo().unwrap();
    assert_eq!(
        studio.project().unwrap().camera_views[0].name,
        "North bridge"
    );
    studio.editor.as_mut().unwrap().undo().unwrap();
    assert_eq!(studio.project().unwrap().camera_views[0].name, "View 1");
    ready(&mut studio);
    Arc::make_mut(&mut studio.view)
        .preview
        .camera
        .as_mut()
        .unwrap()
        .source = Id(999);
    let before = studio.view.camera_workspace.view;
    assert!(studio.camera_views_action(V::Recall(id)).is_err());
    assert_eq!(studio.view.camera_workspace.view, before);
}

#[test]
fn stale_names_and_cancel_never_replace_another_view_or_new_document() {
    let mut studio = inspected_project(false);
    studio.begin_camera_view_name(None).unwrap();
    studio.camera_view_text.set_text("Unsaved name");
    studio.cancel_camera_view_name();
    assert!(studio.project().unwrap().camera_views.is_empty());
    studio.begin_camera_view_name(None).unwrap();
    studio
        .edit(
            "Other edit",
            &[Command::Rename {
                name: "Changed".into(),
            }],
        )
        .unwrap();
    assert!(studio.camera_views_action(V::Save).is_err());
    assert!(studio.project().unwrap().camera_views.is_empty());
    studio.install(Editor::new(Project::new("New project")).unwrap(), None);
    assert!(studio.camera_view_edit.is_none());
    assert!(studio.view.camera_workspace.saved_views.draft.is_none());
}

#[test]
fn selected_views_remain_stable_when_paging_and_deleted_ids_are_never_rebound_to_another_row() {
    let mut studio = inspected_project(false);
    let source = studio.project().unwrap().sources[0].clone();
    for index in 0..12 {
        let view = SavedCameraView::new(
            studio.project().unwrap().next_id().unwrap(),
            format!("Angle {index}"),
            &source,
            CameraPose {
                position: [index as f64, 70.0, 0.0],
                ..Default::default()
            },
        )
        .unwrap();
        studio
            .edit("Add view", &[Command::AddCameraView { view }])
            .unwrap();
    }
    let id = studio.project().unwrap().camera_views[2].id;
    studio.camera_views_action(V::Select(id)).unwrap();
    studio.camera_views_action(V::Page(1)).unwrap();
    assert_eq!(studio.view.camera_workspace.saved_views.selected, Some(id));
    studio.camera_views_action(V::Delete(id)).unwrap();
    studio.reconcile_saved_camera_views();
    assert!(studio.view.camera_workspace.saved_views.selected.is_none());
    assert!(studio.camera_views_action(V::Recall(id)).is_err());
}

#[test]
fn camera_tool_switches_cancel_hidden_drafts_without_editing_the_document() {
    let mut studio = inspected_project(false);
    studio.begin_camera_view_name(None).unwrap();
    let revision = studio.editor.as_ref().unwrap().revision();
    let epoch = studio.transport.epoch();
    for section in launcher::menu::studio::CameraTools::ALL {
        studio.camera_views_action(V::Section(*section)).unwrap();
        assert_eq!(studio.view.camera_workspace.tools, *section);
        assert!(studio.view.camera_workspace.saved_views.draft.is_none());
        assert_eq!(studio.editor.as_ref().unwrap().revision(), revision);
        assert_eq!(studio.transport.epoch(), epoch);
    }
}
