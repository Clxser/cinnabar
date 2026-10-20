use super::*;
use crate::{
    semantic_controls::{
        PendingDeviceFrame, SemanticInputRuntime, SemanticRouteState, SemanticTouchTargets,
        collect_raw_input, finalize_semantic_input_after_ui_authority, route_semantic_input,
    },
    ui_runtime::presentation::observe_mount_jump_input,
};
use bevy::{
    input::{
        InputPlugin,
        gamepad::GamepadAxis,
        keyboard::Key,
        mouse::MouseMotion,
        touch::{TouchInput, TouchPhase},
    },
    prelude::{App, IntoScheduleConfigs, Update},
    window::{CursorGrabMode, WindowEvent},
};
use client_ui::test_support::{form_input_mode, mini_engine_presentation, pack_harness, rasterize};

/// Opens an original form through the production raw-input and UI observation systems.
fn host() -> (App, bevy::prelude::Entity) {
    let mut player = client_state::PlayerRuntime(player_state::PlayerState::new(1));
    let runtime = pack_harness::action_form(&mut player, "Menu", &["Continue"]);
    let mut presentation = mini_engine_presentation();
    presentation.set_server_ui_pack(&form_input_mode::pack());
    let mut app = App::new();
    app.add_plugins(InputPlugin)
        .add_message::<WindowEvent>()
        .init_resource::<Time<Real>>()
        .init_resource::<SemanticInputRuntime>()
        .init_resource::<PendingDeviceFrame>()
        .init_resource::<SemanticRouteState>()
        .init_resource::<SemanticTouchTargets>()
        .init_resource::<client_state::SemanticInputSnapshot>()
        .insert_resource(player)
        .insert_resource(UiRuntimeResource(runtime))
        .insert_resource(UiPresentationResource(presentation))
        .add_systems(
            Update,
            (
                collect_raw_input,
                route_semantic_input,
                drive_server_form_input,
                finalize_semantic_input_after_ui_authority,
                observe_mount_jump_input,
            )
                .chain(),
        );
    app.world_mut().spawn((
        Window::default(),
        CursorOptions {
            grab_mode: CursorGrabMode::Locked,
            visible: false,
            ..Default::default()
        },
        PrimaryWindow,
    ));
    let pad = app.world_mut().spawn(Gamepad::default()).id();
    (app, pad)
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

/// Borrows the finalized action frame produced by the raw-input pipeline.
fn input(app: &App) -> &semantic_input::ActionSnapshot {
    app.world()
        .resource::<client_state::SemanticInputSnapshot>()
        .snapshot()
        .unwrap()
}

/// Applies controller look activity before the production raw sampler runs.
fn controller(app: &mut App, pad: bevy::prelude::Entity, value: f32) {
    app.world_mut()
        .get_mut::<Gamepad>(pad)
        .unwrap()
        .analog_mut()
        .set(GamepadAxis::RightStickX, value);
    app.update();
}

#[test]
fn mouse_activity_updates_form_mode_while_capture_is_released() {
    let (mut app, pad) = host();
    let mouse = pixels(&mut app);
    controller(&mut app, pad, 0.8);
    let gamepad = pixels(&mut app);
    assert!(gamepad != mouse, "controller use must reveal the image");
    let (window, cursor) = app
        .world_mut()
        .query_filtered::<(bevy::prelude::Entity, &CursorOptions), With<PrimaryWindow>>()
        .single(app.world())
        .unwrap();
    assert_eq!(cursor.grab_mode, CursorGrabMode::None);
    app.world_mut().write_message(MouseMotion {
        delta: bevy::math::Vec2::X,
    });
    app.update();
    assert_eq!(
        input(&app).input_mode,
        semantic_input::InputMode::KeyboardMouse
    );
    assert!(
        pixels(&mut app) == mouse,
        "mouse movement must hide the image"
    );

    controller(&mut app, pad, 0.9);
    assert!(pixels(&mut app) == gamepad);
    let key = KeyboardInput {
        key_code: KeyCode::KeyW,
        logical_key: Key::Character("w".into()),
        state: ButtonState::Pressed,
        text: Some("w".into()),
        repeat: false,
        window,
    };
    let button = MouseButtonInput {
        button: MouseButton::Left,
        state: ButtonState::Pressed,
        window,
    };
    app.world_mut().write_message(key.clone());
    app.world_mut()
        .write_message(WindowEvent::KeyboardInput(key));
    app.world_mut().write_message(button);
    app.world_mut()
        .write_message(WindowEvent::MouseButtonInput(button));
    app.update();
    let snapshot = input(&app);
    assert_eq!(
        snapshot.input_mode,
        semantic_input::InputMode::KeyboardMouse
    );
    assert_eq!(snapshot.movement, [0.0; 2]);
    assert_eq!(snapshot.look_delta, [0.0; 2]);
    assert!(
        snapshot
            .phases
            .iter()
            .all(|phase| !phase.pressed && !phase.held)
    );
    assert!(
        pixels(&mut app) == mouse,
        "desktop buttons must update only the mode"
    );
    controller(&mut app, pad, 0.88);
    let wheel = MouseWheel {
        unit: MouseScrollUnit::Line,
        x: 0.0,
        y: 1.0,
        window,
        phase: TouchPhase::Moved,
    };
    app.world_mut().write_message(wheel);
    app.world_mut()
        .write_message(WindowEvent::MouseWheel(wheel));
    app.update();
    assert_eq!(
        input(&app).input_mode,
        semantic_input::InputMode::KeyboardMouse
    );
    assert!(
        input(&app)
            .phases
            .iter()
            .all(|phase| !phase.pressed && !phase.held)
    );
    assert!(pixels(&mut app) == mouse);
    controller(&mut app, pad, 0.85);
    app.world_mut().write_message(TouchInput {
        phase: TouchPhase::Started,
        position: bevy::math::Vec2::new(4.0, 4.0),
        window,
        force: None,
        id: 1,
    });
    app.update();
    assert_eq!(input(&app).input_mode, semantic_input::InputMode::Touch);
    assert!(
        pixels(&mut app) == mouse,
        "unassigned touches must update the mode"
    );
}

#[test]
fn settings_reissue_keeps_gamepad_image_visible_without_new_input() {
    let (mut app, pad) = host();
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
    controller(&mut app, pad, 0.8);
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
