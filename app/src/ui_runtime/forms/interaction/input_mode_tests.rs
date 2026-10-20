use super::*;
use crate::{
    semantic_controls::SemanticInputRuntime, ui_runtime::presentation::observe_mount_jump_input,
};
use bevy::prelude::{App, IntoScheduleConfigs, Update};
use client_ui::test_support::{form_input_mode, mini_engine_presentation, pack_harness, rasterize};
use semantic_input::{ControllerFrame, DeviceFrame, KeyboardMouseFrame, TouchContact};

/// Opens an original form with the app-wide semantic input and UI observation boundary.
fn host() -> App {
    let mut player = client_state::PlayerRuntime(player_state::PlayerState::new(1));
    let runtime = pack_harness::action_form(&mut player, "Menu", &["Continue"]);
    let mut presentation = mini_engine_presentation();
    presentation.set_server_ui_pack(&form_input_mode::pack());
    let mut app = App::new();
    app.add_message::<KeyboardInput>()
        .add_message::<MouseWheel>()
        .add_message::<MouseButtonInput>()
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<AccumulatedMouseMotion>()
        .init_resource::<Time<Real>>()
        .init_resource::<SemanticInputRuntime>()
        .init_resource::<client_state::SemanticInputSnapshot>()
        .insert_resource(player)
        .insert_resource(UiRuntimeResource(runtime))
        .insert_resource(UiPresentationResource(presentation))
        .add_systems(
            Update,
            (drive_server_form_input, observe_mount_jump_input).chain(),
        );
    app.world_mut()
        .spawn((Window::default(), CursorOptions::default(), PrimaryWindow));
    app
}

/// Rebuilds through production presentation and returns its painted pixels.
fn pixels(app: &mut App) -> Vec<u8> {
    app.world_mut().resource_scope(
        |world, mut presentation: bevy::prelude::Mut<UiPresentationResource>| {
            let frame = presentation
                .build(
                    world.resource::<client_state::PlayerRuntime>(),
                    world.resource::<UiRuntimeResource>(),
                    0,
                    [640, 360],
                    ui::DpiScale::new(1.0).unwrap(),
                )
                .unwrap();
            rasterize(&frame).into_raw()
        },
    )
}

/// Publishes a device sample through the same router and snapshot used by gameplay.
fn input(app: &mut App, frame: DeviceFrame) {
    let snapshot = app
        .world_mut()
        .resource_mut::<SemanticInputRuntime>()
        .route_and_finalize(frame)
        .unwrap();
    app.world_mut()
        .resource_mut::<client_state::SemanticInputSnapshot>()
        .replace(snapshot);
    app.update();
}

/// Supplies a right-stick sample to the app-wide router.
fn controller_frame(right_stick: f32) -> DeviceFrame {
    DeviceFrame {
        controllers: vec![ControllerFrame {
            device_id: 1,
            axes: [0.0, 0.0, right_stick, 0.0, 0.0, 0.0, 0.0, 0.0],
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[test]
fn controller_activity_reveals_gamepad_image_until_mouse_or_touch_use() {
    use semantic_input::InputMode::{GamePad, KeyboardMouse, Touch};
    let mut app = host();
    let mouse = pixels(&mut app);
    let mut idle = controller_frame(0.0);
    idle.keyboard_mouse = Some(KeyboardMouseFrame::default());
    let mut mouse_input = controller_frame(0.8);
    mouse_input.keyboard_mouse = Some(KeyboardMouseFrame {
        mouse_motion: [1.0, 0.0],
        ..Default::default()
    });
    let touch = DeviceFrame {
        touches: vec![TouchContact {
            contact_id: 1,
            activity_sequence: 0,
            position: [0.25, 0.5],
            delta: [0.0, 0.0],
            hit_id: None,
        }],
        ..Default::default()
    };
    for (frame, mode) in [
        (idle, KeyboardMouse),
        (controller_frame(0.8), GamePad),
        (controller_frame(0.8), GamePad),
        (mouse_input, KeyboardMouse),
        (controller_frame(0.9), GamePad),
        (touch, Touch),
    ] {
        input(&mut app, frame);
        assert_eq!(
            app.world()
                .resource::<client_state::SemanticInputSnapshot>()
                .snapshot()
                .unwrap()
                .input_mode,
            mode,
            "the app-wide router must select the active device"
        );
        assert_eq!(pixels(&mut app) != mouse, mode == GamePad);
    }
}

#[test]
fn settings_reissue_keeps_gamepad_image_visible_without_new_input() {
    let mut app = host();
    let entry = app
        .world()
        .resource::<UiRuntimeResource>()
        .server_forms()
        .active()
        .unwrap()
        .clone();
    let event = protocol::FormRequestEvent {
        form_id: entry.form_id,
        kind: protocol::FormKind::ServerSettings,
        title: entry.title,
        json: "{}".into(),
        model: entry.model,
    };
    {
        let mut runtime = app.world_mut().resource_mut::<UiRuntimeResource>();
        let session = runtime.session_id();
        let store = runtime.server_forms_mut();
        store.select_settings_form(true);
        store.admit(event.clone(), 2, session, false);
    }
    let mouse = pixels(&mut app);
    input(&mut app, controller_frame(0.8));
    let gamepad = pixels(&mut app);
    assert!(gamepad != mouse, "controller use must reveal the image");
    {
        let mut runtime = app.world_mut().resource_mut::<UiRuntimeResource>();
        let session = runtime.session_id();
        runtime.server_forms_mut().admit(event, 3, session, false);
    }
    assert!(
        pixels(&mut app) == gamepad,
        "reissuing the form must retain the device binding without another input event"
    );
}
