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
fn held_stick_navigates_the_first_rendered_form_frame() {
    use bevy::input::gamepad::GamepadAxis;

    let (mut app, pad) = host();
    app.world_mut()
        .get_mut::<Gamepad>(pad)
        .unwrap()
        .analog_mut()
        .set(GamepadAxis::LeftStickY, -0.9);
    app.update();
    control_pack(
        &mut app,
        r#"{"type":"button","focus_enabled":true,"size":[100,30],"collection_index":0}"#,
    );
    app.update();
    assert!(
        app.world()
            .resource::<UiRuntimeResource>()
            .server_forms()
            .engine()
            .view
            .focused
            .is_some(),
        "a stick held before rendering must navigate when the form frame becomes available"
    );
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
    // A delivered touch movement owns presentation without changing this key's source.
    app.init_resource::<crate::ui_runtime::touch_pointer::TouchPointer>();
    app.world_mut()
        .resource_mut::<crate::ui_runtime::touch_pointer::TouchPointer>()
        .delivered = true;
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
fn cancelled_touch_scroll_ending_over_a_toggle_leaves_it_unchanged() {
    use crate::ui_runtime::touch_pointer::{TouchPointer, emulate_ui_pointer};
    use bevy::{
        input::touch::{TouchInput, TouchPhase},
        prelude::PreUpdate,
    };

    for end in [TouchPhase::Ended, TouchPhase::Canceled] {
        let (mut app, _) = host();
        app.add_message::<TouchInput>()
            .init_resource::<TouchPointer>()
            .add_systems(PreUpdate, emulate_ui_pointer);
        control_pack(
            &mut app,
            r#"{"type":"scroll_view","size":[200,100],
                "scroll_view_port":"viewport","scroll_content":"content",
                "scrollbar_track":"track","scrollbar_box":"box",
                "gesture_control_enabled":true,"scrollbar_touch_button":"button.scroll_touch",
                "button_mappings":[{"from_button_id":"button.menu_select","to_button_id":"button.scroll_touch","mapping_type":"pressed"}],
                "controls":[{"viewport":{"type":"panel","size":[200,100],
                    "anchor_from":"top_left","anchor_to":"top_left","controls":[{
                    "content":{"type":"panel","size":[200,200],
                        "anchor_from":"top_left","anchor_to":"top_left","controls":[{
                        "toggle":{"type":"toggle","size":[40,20],"offset":[50,60],
                            "anchor_from":"top_left","anchor_to":"top_left","toggle_name":"test_toggle",
                            "button_mappings":[{"from_button_id":"button.menu_select","to_button_id":"button.toggle","mapping_type":"pressed"}]}
                    }]}
                }]}},
                {"track":{"type":"scroll_track","size":[4,100],"offset":[196,0],
                    "anchor_from":"top_left","anchor_to":"top_left"}},
                {"box":{"type":"scrollbar_box","size":[4,20],"offset":[196,0],
                    "anchor_from":"top_left","anchor_to":"top_left","draggable":"vertical"}}]}
            "#,
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
        let target = bevy::math::Vec2::new(
            frame.origin[0] + (hit.rect.x + 2.0) as f32 * frame.scale,
            frame.origin[1] + (hit.rect.y + 2.0) as f32 * frame.scale,
        );
        let start = target - bevy::math::Vec2::Y * 40.0 * frame.scale;
        let window = app
            .world_mut()
            .query_filtered::<bevy::prelude::Entity, With<PrimaryWindow>>()
            .single(app.world())
            .unwrap();
        for (phase, position) in [
            (TouchPhase::Started, start),
            (TouchPhase::Moved, target),
            (end, target),
        ] {
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
            if phase == TouchPhase::Started {
                assert!(
                    engine.scroll_touch.is_some(),
                    "the gesture must start in the scroll view"
                );
            }
            if phase == TouchPhase::Moved {
                let (view, _) = engine.scroll_touch.as_ref().unwrap();
                assert!(
                    !engine.view.clone().end_scroll_touch(view),
                    "the scroll must travel beyond tap slop"
                );
            }
            let checked = engine
                .view
                .components
                .bag(&key)
                .and_then(|bag| bag.get("#toggle_state"))
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            assert!(
                !checked,
                "a cancelled scroll must not change the toggle: {phase:?}"
            );
        }
    }
}
