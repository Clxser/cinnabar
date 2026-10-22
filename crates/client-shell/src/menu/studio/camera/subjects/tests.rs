use super::*;
use crate::menu::studio::camera::authoring_tests::inspected_project;
use launcher::menu::studio::{CameraTools, CameraViewsAction, Toolbox};
use recording_studio::camera::{
    ActorPose, CameraPose, CameraSubjectFilter, CameraTarget, CameraTargetKind,
};

/// Identical names deliberately require stable captured spawn references to distinguish subjects.
fn target(id: u64, kind: CameraTargetKind) -> CameraTarget {
    CameraTarget {
        runtime_id: id,
        unique_id: id as i64 * 10,
        spawn_revision: 1,
        name: Arc::from("Alpha"),
        kind,
        entity_type: None,
        view: Default::default(),
        pose: ActorPose {
            position: [0.0, 64.0, 4.0],
            velocity: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
            eye_offset: None,
        },
        teleported: false,
    }
}

/// The metadata fixture publishes a ready source frame without any recording decoder or GPU.
fn ready(studio: &mut Studio, targets: Arc<[CameraTarget]>) {
    let epoch = [
        studio.document,
        studio.editor.as_ref().unwrap().revision(),
        studio.transport.epoch(),
    ];
    let preview = &mut Arc::make_mut(&mut studio.view).preview;
    preview.ready = true;
    preview.epoch = Some(epoch);
    let camera = preview.camera.as_mut().unwrap();
    camera.epoch = epoch;
    camera.targets = targets;
    studio.reconcile_camera_subjects();
}

/// Result actions retain source/frame identity in addition to a captured spawn lifetime.
fn selection(studio: &Studio, target: &CameraTarget) -> S {
    let preview = studio.view.preview.camera.as_ref().unwrap();
    S::Select {
        source: preview.source,
        epoch: preview.epoch,
        target: target.reference(),
    }
}

#[test]
fn search_and_selection_preserve_authored_target_and_reject_reused_spawn_references() {
    let mut studio = inspected_project(true);
    let first = target(1, CameraTargetKind::Player);
    let second = target(2, CameraTargetKind::Player);
    let shot = studio.camera_shot().unwrap();
    let shot = CameraShot::from_preview(
        shot.id,
        shot.span,
        CameraRigKind::Follow,
        CameraPose::default(),
        Some(&first),
    )
    .unwrap();
    studio
        .edit("Follow first spawn", &[Command::ReplaceShot { shot }])
        .unwrap();
    ready(
        &mut studio,
        Arc::from([
            first.clone(),
            second.clone(),
            target(3, CameraTargetKind::Entity),
            target(4, CameraTargetKind::Unknown),
        ]),
    );
    let project = Arc::clone(studio.project().unwrap());
    let revision = studio.editor.as_ref().unwrap().revision();
    studio.subject_text.set_text("ALP");
    studio.publish_camera_subject_search();
    assert_eq!(
        studio
            .view
            .camera_workspace
            .subject_results
            .as_ref()
            .unwrap()
            .len(),
        2
    );
    let action = selection(&studio, &second);
    studio.camera_subjects_action(action).unwrap();
    assert_eq!(studio.view.camera_target, Some(second.reference()));
    assert!(Arc::ptr_eq(&project, studio.project().unwrap()));
    assert_eq!(studio.editor.as_ref().unwrap().revision(), revision);
    let mut respawned = second.clone();
    respawned.spawn_revision += 1;
    ready(&mut studio, Arc::from([first, respawned.clone()]));
    assert!(studio.camera_subjects_action(action).is_err());
    assert_eq!(studio.view.camera_target, Some(second.reference()));
    studio
        .camera_subjects_action(selection(&studio, &respawned))
        .unwrap();
    assert_eq!(studio.view.camera_target, Some(respawned.reference()));
    assert!(Arc::ptr_eq(&project, studio.project().unwrap()));
}

#[test]
fn query_cache_reuses_unchanged_publications_and_old_take_or_seek_actions_are_rejected() {
    let mut studio = inspected_project(false);
    let player = target(2, CameraTargetKind::Player);
    ready(&mut studio, Arc::from([player.clone()]));
    studio.subject_text.set_text("alpha");
    studio.publish_camera_subject_search();
    let view = Arc::clone(&studio.view);
    studio.reconcile_camera_subjects();
    assert!(Arc::ptr_eq(&view, &studio.view));
    studio.publish_camera_subject_search();
    assert!(Arc::ptr_eq(&view, &studio.view));
    let old = selection(&studio, &player);
    studio
        .transport
        .seek(MediaTime(750_000), MediaTime(2_000_000));
    ready(&mut studio, Arc::from([player.clone()]));
    assert!(studio.camera_subjects_action(old).is_err());
    let old = selection(&studio, &player);
    Arc::make_mut(&mut studio.view)
        .preview
        .camera
        .as_mut()
        .unwrap()
        .source = Id(99);
    assert!(studio.camera_subjects_action(old).is_err());
}

#[test]
fn query_paging_and_legacy_all_entity_fallback_never_dirty_or_retarget_the_project() {
    let mut studio = inspected_project(false);
    let targets: Arc<[_]> = (1..=10)
        .map(|id| {
            target(
                id,
                if id <= 8 {
                    CameraTargetKind::Player
                } else {
                    CameraTargetKind::Unknown
                },
            )
        })
        .collect();
    ready(&mut studio, targets);
    let revision = studio.editor.as_ref().unwrap().revision();
    let epoch = studio.transport.epoch();
    studio.subject_text.set_text("alpha");
    studio.publish_camera_subject_search();
    assert_eq!(
        studio
            .view
            .camera_workspace
            .subject_results
            .as_ref()
            .unwrap()
            .len(),
        8
    );
    studio.camera_subjects_action(S::Page(1)).unwrap();
    assert_eq!(
        studio.view.camera_workspace.subject_first,
        CAMERA_SUBJECT_ROWS
    );
    studio.toggle_camera_subject_filter();
    assert_eq!(
        studio.view.camera_workspace.subject_filter,
        CameraSubjectFilter::All
    );
    assert_eq!(
        studio
            .view
            .camera_workspace
            .subject_results
            .as_ref()
            .unwrap()
            .len(),
        10
    );
    studio.camera_subjects_action(S::Clear).unwrap();
    assert!(studio.view.camera_workspace.subject_search.is_empty());
    assert_eq!(studio.editor.as_ref().unwrap().revision(), revision);
    assert_eq!(studio.transport.epoch(), epoch);
}

#[test]
fn camera_creation_and_view_focus_follow_visible_sections_but_selected_properties_stay_independent()
{
    let mut studio = inspected_project(true);
    let workspace = &mut Arc::make_mut(&mut studio.view).workspace;
    workspace.toolbox = Toolbox::Cameras;
    workspace.media_hidden = false;
    for section in CameraTools::ALL {
        Arc::make_mut(&mut studio.view).camera_workspace.tools = *section;
        let actions = studio.camera_focus_actions();
        assert_eq!(
            actions.contains(&Action::CreateCamera(CameraRigKind::Free)),
            *section == CameraTools::Shots
        );
        assert_eq!(
            actions.contains(&Action::CameraViews(CameraViewsAction::New)),
            *section == CameraTools::Views
        );
        assert!(actions.contains(&Action::ToggleCameraSmoothing));
        for hidden in [true, false] {
            Arc::make_mut(&mut studio.view).workspace.media_hidden = hidden;
            let actions = studio.camera_focus_actions();
            assert_eq!(
                actions.iter().any(|action| matches!(
                    action,
                    Action::CameraViews(CameraViewsAction::Section(_))
                )),
                !hidden
            );
            assert!(actions.contains(&Action::ToggleCameraSmoothing));
        }
    }
    Arc::make_mut(&mut studio.view).workspace.preview_only = true;
    let actions = studio.camera_focus_actions();
    assert!(
        !actions
            .iter()
            .any(|action| matches!(action, Action::CreateCamera(_) | Action::CameraViews(_)))
    );
    Arc::make_mut(&mut studio.view).selected = None;
    assert!(
        !studio
            .camera_focus_actions()
            .contains(&Action::ToggleCameraSmoothing)
    );
}
