use super::*;
use bevy::prelude::{App, IntoScheduleConfigs, Update};
use client_ui::test_support::{form_input_mode, mini_engine_presentation, pack_harness, rasterize};

/// Creates the host input system with an open form and an idle connected controller.
fn host() -> (App, bevy::prelude::Entity) {
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
        .insert_resource(player)
        .insert_resource(UiRuntimeResource(runtime))
        .insert_resource(UiPresentationResource(presentation))
        .add_systems(
            Update,
            (track_server_form_input_mode, drive_server_form_input).chain(),
        );
    app.world_mut()
        .spawn((Window::default(), CursorOptions::default(), PrimaryWindow));
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

#[test]
fn controller_press_reveals_gamepad_image_until_mouse_use() {
    let (mut app, pad) = host();
    let mouse = pixels(&mut app);
    app.update();
    assert!(pixels(&mut app) == mouse, "an idle controller is not input");
    app.world_mut()
        .get_mut::<Gamepad>(pad)
        .unwrap()
        .digital_mut()
        .press(GamepadButton::DPadDown);
    app.update();
    let gamepad = pixels(&mut app);
    assert!(gamepad != mouse, "controller use must reveal the image");
    app.world_mut()
        .get_mut::<Gamepad>(pad)
        .unwrap()
        .digital_mut()
        .clear();
    app.update();
    assert!(pixels(&mut app) == gamepad, "idle frames retain the mode");
    app.world_mut()
        .resource_mut::<AccumulatedMouseMotion>()
        .delta = bevy::math::Vec2::X;
    app.update();
    assert!(pixels(&mut app) == mouse, "mouse use must hide the image");
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
    app.world_mut()
        .get_mut::<Gamepad>(pad)
        .unwrap()
        .digital_mut()
        .press(GamepadButton::DPadDown);
    app.update();
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
