use super::*;
use bevy::prelude::{App, Update};
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
        .add_systems(Update, drive_server_form_input);
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

/// Installs an original control while retaining the host's form and presentation systems.
fn control_pack(app: &mut App, control: &str) {
    let json = format!(
        r#"{{"namespace":"server_form","long_form":{{"type":"panel","factory":null,"collection_name":"","size":[200,100],"controls":[{{"control":{control}}}]}}}}"#
    );
    app.world_mut()
        .resource_mut::<UiPresentationResource>()
        .set_server_ui_pack(&client_ui::ui_runtime::presentation::forms::ServerUiPack {
            ui_layers: vec![vec![("ui/server_form.json".into(), json.into_bytes())]],
            ..Default::default()
        });
    pixels(app);
}

#[test]
fn controller_mapping_receives_gamepad_mode_and_keyboard_keeps_focus_navigation() {
    let (mut app, pad) = host();
    control_pack(
        &mut app,
        r#"{"type":"button","focus_enabled":true,"size":[100,30],"collection_index":0,
        "button_mappings":[{"from_button_id":"button.menu_ok","to_button_id":"button.form_button_click","mapping_type":"focused","input_mode_condition":"gamepad"}]}"#,
    );
    let window = app
        .world_mut()
        .query_filtered::<bevy::prelude::Entity, With<PrimaryWindow>>()
        .single(app.world())
        .unwrap();
    app.world_mut().write_message(KeyboardInput {
        key_code: KeyCode::Tab,
        state: ButtonState::Pressed,
        logical_key: bevy::input::keyboard::Key::Tab,
        text: None,
        repeat: false,
        window,
    });
    app.update();
    assert!(
        app.world()
            .resource::<UiRuntimeResource>()
            .server_forms()
            .engine()
            .view
            .focused
            .is_some()
    );
    app.world_mut()
        .get_mut::<Gamepad>(pad)
        .unwrap()
        .digital_mut()
        .press(GamepadButton::South);
    app.update();
    assert!(
        app.world()
            .resource::<UiRuntimeResource>()
            .server_forms()
            .active()
            .is_none(),
        "the gamepad-only mapping must submit"
    );
}

#[test]
fn touch_toggle_changes_on_release_through_the_host_pointer_adapter() {
    use crate::ui_runtime::touch_pointer::{TouchPointer, emulate_ui_pointer};
    use bevy::{
        input::touch::{TouchInput, TouchPhase},
        prelude::PreUpdate,
    };
    let (mut app, _) = host();
    app.add_message::<TouchInput>()
        .init_resource::<TouchPointer>()
        .add_systems(PreUpdate, emulate_ui_pointer);
    control_pack(
        &mut app,
        r#"{"type":"toggle","size":[100,30],"toggle_name":"test_toggle",
        "button_mappings":[{"from_button_id":"button.menu_select","to_button_id":"button.toggle","mapping_type":"pressed"}]}"#,
    );
    let runtime = app.world().resource::<UiRuntimeResource>();
    let identity = runtime.server_forms().active().unwrap().identity;
    let frame = app
        .world()
        .resource::<UiPresentationResource>()
        .form_engine_frame(identity)
        .unwrap();
    let hit = frame
        .hits
        .iter()
        .find(|hit| hit.kind == json_ui::HitKind::Toggle)
        .unwrap();
    let key = hit.key.clone();
    let position = bevy::math::Vec2::new(
        frame.origin[0] + (hit.rect.x + 2.0) as f32 * frame.scale,
        frame.origin[1] + (hit.rect.y + 2.0) as f32 * frame.scale,
    );
    let window = app
        .world_mut()
        .query_filtered::<bevy::prelude::Entity, With<PrimaryWindow>>()
        .single(app.world())
        .unwrap();
    for (phase, checked) in [(TouchPhase::Started, false), (TouchPhase::Ended, true)] {
        app.world_mut().write_message(TouchInput {
            phase,
            position,
            window,
            force: None,
            id: 1,
        });
        app.update();
        let engine = app
            .world()
            .resource::<UiRuntimeResource>()
            .server_forms()
            .engine();
        assert_eq!(engine.view.input_mode, json_ui::InputMode::Touch);
        let actual = engine
            .view
            .components
            .bag(&key)
            .and_then(|bag| bag.get("#toggle_state"))
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        assert_eq!(actual, checked, "touch activation belongs to release");
    }
}
