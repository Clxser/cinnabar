use super::*;
use crate::menu::studio::camera::authoring_tests::inspected_project;
use recording_studio::{effect::Easing, tracks::Track};

#[test]
fn entry_controls_save_one_undoable_edit_and_preserve_rig_keys_and_lens() {
    let mut studio = inspected_project(true);
    let before = studio.camera_shot().unwrap();
    let revision = studio.editor.as_ref().unwrap().revision();
    studio
        .camera_blend_action(CameraBlendAction::Toggle)
        .unwrap();
    let after = studio.camera_shot().unwrap();
    assert_eq!(after.mode, before.mode);
    assert_eq!(after.fov_curve, before.fov_curve);
    assert_eq!(after.span, before.span);
    assert_eq!(
        after.blend_in,
        Some(CameraBlend {
            duration: CameraBlend::DEFAULT_DURATION,
            easing: Easing::Smooth
        })
    );
    assert_eq!(studio.editor.as_ref().unwrap().revision(), revision + 1);
    studio.editor.as_mut().unwrap().undo().unwrap();
    assert_eq!(studio.camera_shot().unwrap(), before);
    studio.editor.as_mut().unwrap().redo().unwrap();
    assert_eq!(studio.camera_shot().unwrap(), after);
    studio
        .camera_blend_action(CameraBlendAction::CycleEasing)
        .unwrap();
    assert_eq!(
        studio.camera_shot().unwrap().blend_in.unwrap().easing,
        Easing::Smooth.next()
    );
    studio
        .camera_blend_action(CameraBlendAction::Toggle)
        .unwrap();
    assert!(studio.camera_shot().unwrap().blend_in.is_none());
}

#[test]
fn duration_controls_use_the_saved_output_frame_and_clamp_without_empty_blends() {
    let mut studio = inspected_project(true);
    studio
        .camera_blend_action(CameraBlendAction::Toggle)
        .unwrap();
    studio
        .camera_blend_action(CameraBlendAction::Duration(1))
        .unwrap();
    assert_eq!(
        studio.camera_shot().unwrap().blend_in.unwrap().duration,
        MediaTime(CameraBlend::DEFAULT_DURATION.0 + CameraBlend::DURATION_STEP.0)
    );
    Arc::make_mut(&mut studio.view).camera_fine = true;
    let before = studio.camera_shot().unwrap().blend_in.unwrap().duration;
    let frame = studio.project().unwrap().export.fps.frame_time(1).unwrap();
    studio
        .camera_blend_action(CameraBlendAction::Duration(1))
        .unwrap();
    assert_eq!(
        studio.camera_shot().unwrap().blend_in.unwrap().duration,
        MediaTime(before.0 + frame.0)
    );
    Arc::make_mut(&mut studio.view).camera_fine = false;
    for _ in 0..12 {
        studio
            .camera_blend_action(CameraBlendAction::Duration(-1))
            .unwrap();
    }
    assert_eq!(
        studio.camera_shot().unwrap().blend_in.unwrap().duration,
        MediaTime(1)
    );
    let revision = studio.editor.as_ref().unwrap().revision();
    studio
        .camera_blend_action(CameraBlendAction::Duration(0))
        .unwrap();
    studio
        .camera_blend_action(CameraBlendAction::Duration(-1))
        .unwrap();
    assert_eq!(studio.editor.as_ref().unwrap().revision(), revision);
    for _ in 0..12 {
        studio
            .camera_blend_action(CameraBlendAction::Duration(1))
            .unwrap();
    }
    let shot = studio.camera_shot().unwrap();
    assert_eq!(shot.blend_in.unwrap().duration, shot.span.duration());
}

#[test]
fn locked_entry_controls_reject_before_canceling_transient_inspection() {
    let mut studio = inspected_project(true);
    let before = Arc::clone(studio.project().unwrap());
    let inspected = studio.view.camera_workspace.view;
    let revision = studio.editor.as_ref().unwrap().revision();
    Arc::make_mut(&mut studio.view)
        .workspace
        .timeline
        .locked
        .insert(Track::Camera);
    for action in [
        CameraBlendAction::Toggle,
        CameraBlendAction::Duration(1),
        CameraBlendAction::CycleEasing,
    ] {
        assert!(
            studio
                .camera_blend_action(action)
                .unwrap_err()
                .contains("Unlock")
        );
        assert!(Arc::ptr_eq(studio.project().unwrap(), &before));
        assert_eq!(studio.editor.as_ref().unwrap().revision(), revision);
        assert_eq!(studio.view.camera_workspace.view, inspected);
    }
}
