use std::sync::{Arc, OnceLock};

use json_ui::{Catalog, Context, DataSource, FormModel, ViewState};
use serde_json::{Value, json};
use ui::{DpiScale, IconRef, SafeArea, TextLayoutCache, UiNode, UiVisual};

use super::{
    FormEngine, ScreenArt,
    screen::{PaintError, PaintFrame, ScreenRegistry},
};
use crate::{
    cached_screen::CachedScreen,
    frame::EngineFrame,
    sources::{ImageWork, RemoteImage, RemoteSource},
    test_support::fixture_font,
    text_metrics::TextMetrics,
};

struct Offline;
impl RemoteSource for Offline {
    /// Synthetic texture readiness never starts a download.
    fn state(&self, _: &str) -> RemoteImage {
        RemoteImage::Loading
    }
}

/// Supplies the ordinary action-form controller input without external content.
fn model() -> FormModel {
    FormModel::Action(Default::default())
}

/// Installs one original root for both the generic and public form entry paths.
fn catalog(root: Value, mut animations: Value) -> Catalog {
    animations["namespace"] = json!("fixture");
    animations["root"] = root.clone();
    let reference = json_ui::form_template(&model());
    let (namespace, name) = reference.split_once('.').unwrap();
    let mut form = json!({"namespace":namespace});
    form[name] = root;
    let mut catalog = Catalog::default();
    catalog.overlay_text("ui/fixture.json", &animations.to_string());
    catalog.overlay_text("ui/form_fixture.json", &form.to_string());
    catalog
}

/// Creates a real engine with synthetic catalog/font carriers and offline artwork.
fn engine(catalog: Catalog) -> FormEngine {
    static REGISTRY: OnceLock<ScreenRegistry> = OnceLock::new();
    let bytes = assets::encode_ui_catalog([1; 32], &[], &[], &[], &[]).unwrap();
    FormEngine::new(
        Arc::new(assets::RuntimeUiAssets::decode(&bytes).unwrap()),
        catalog,
        0,
        REGISTRY.get_or_init(|| ScreenRegistry::new(Context::desktop())),
        Arc::new(Offline),
        ImageWork::inline(),
    )
}

/// Paints one supplied public entry and closes its animation frame.
fn paint(
    engine: &mut FormEngine,
    now: f64,
    draw: impl FnOnce(&mut FormEngine, &mut PaintFrame<'_>) -> Result<Option<EngineFrame>, PaintError>,
) -> (EngineFrame, Vec<UiNode>) {
    let font = fixture_font();
    let metrics = TextMetrics::for_viewport([400, 200], DpiScale::new(1.0).unwrap(), None);
    let px = metrics.scale.get() * ui::FONT_DESIGN_PIXEL_TEXELS as f32;
    let mut layouts = TextLayoutCache::new(16, 1024 * 1024);
    let mut nodes = Vec::new();
    let mut next = 1;
    let result = draw(
        engine,
        &mut PaintFrame {
            layouts: &mut layouts,
            font: &font,
            metrics,
            solid_page: 0,
            safe_area: SafeArea::ZERO,
            content: [200.0 * px, 100.0 * px],
            translate: &|_| None,
            language: [0; 3],
            now,
            nodes: &mut nodes,
            next: &mut next,
            renderers: &mut [],
        },
    )
    .unwrap()
    .unwrap();
    engine.animator().end_frame();
    (result, nodes)
}

/// Makes each interaction state own a distinct input target beneath a geometry control.
fn state_catalog() -> Catalog {
    catalog(
        json!({
            "type":"button","size":[100,50],"anchor_from":"top_left","anchor_to":"top_left",
            "offset":"@fixture.still","default_control":"idle","hover_control":"over","pressed_control":"down",
            "controls":[
                {"idle":{"type":"panel","controls":[{"idle_hit":{"type":"button","controls":[{"fill":{"type":"image","color":[1,0,0]}}]}}]}},
                {"over":{"type":"panel","controls":[{"hover_hit":{"type":"button","controls":[{"fill":{"type":"image","color":[0,1,0]}}]}}]}},
                {"down":{"type":"panel","controls":[{"press_hit":{"type":"button","controls":[{"fill":{"type":"image","color":[0,0,1]}}]}}]}}
            ]
        }),
        json!({"still":{"anim_type":"offset","from":[0,0],"to":[0,0],"duration":1}}),
    )
}

/// Checks the currently actionable target rather than the layout's internal state tree.
fn assert_state_hits(frame: &EngineFrame, selected: &str) {
    for name in ["idle_hit", "hover_hit", "press_hit"] {
        assert_eq!(
            frame
                .hits
                .iter()
                .any(|hit| hit.name == name && hit.contains([5.0, 5.0])),
            name == selected,
            "{name}, selected {selected}"
        );
    }
}

#[test]
fn generic_cached_screen_keeps_actual_hover_and_press_hits() {
    let mut engine = engine(state_catalog());
    let catalog = Arc::clone(engine.catalog());
    let mut cached = CachedScreen::default();
    for (index, selected) in ["idle_hit", "hover_hit", "press_hit"]
        .into_iter()
        .enumerate()
    {
        let view = ViewState {
            hovered: (index == 1).then(|| "/root".into()),
            pressed: (index == 2).then(|| "/root".into()),
            ..ViewState::default()
        };
        let (frame, _) = paint(&mut engine, index as f64, |engine, frame| {
            let px = frame.metrics.scale.get() * ui::FONT_DESIGN_PIXEL_TEXELS as f32;
            engine.draw(
                ScreenArt {
                    view: Some(&view),
                    ..ScreenArt::default()
                },
                frame,
                |env, root| {
                    cached.render_with(
                        "fixture.root",
                        &catalog,
                        &Context::desktop(),
                        DataSource::new(),
                        (root, px, [0; 3]),
                        env,
                        &view,
                    )
                },
            )
        });
        assert_state_hits(&frame, selected);
    }
}

#[test]
fn generic_draw_keeps_the_prepared_input_snapshot() {
    let mut engine = engine(state_catalog());
    let catalog = Arc::clone(engine.catalog());
    let mut cached = CachedScreen::default();
    let view = ViewState::default();
    let mut source_hits = None;
    let (frame, _) = paint(&mut engine, 0.0, |engine, frame| {
        let px = frame.metrics.scale.get() * ui::FONT_DESIGN_PIXEL_TEXELS as f32;
        engine.draw(ScreenArt::default(), frame, |env, root| {
            let source = cached.render_with(
                "fixture.root",
                &catalog,
                &Context::desktop(),
                DataSource::new(),
                (root, px, [0; 3]),
                env,
                &view,
            )?;
            source_hits = Some(Arc::clone(&source.hits));
            Some(source)
        })
    });
    assert!(Arc::ptr_eq(source_hits.as_ref().unwrap(), &frame.hits));
    assert_state_hits(&frame, "idle_hit");
}

#[test]
fn generic_omitted_controls_keep_the_paint_lifecycle() {
    let mut engine = engine(catalog(
        json!({"type":"image","size":[40,20],"color":[1,0,0],"offset":"@fixture.move"}),
        json!({"move":{"anim_type":"offset","from":[0,0],"to":[20,0],"duration":1,"end_event":"fixture.done"}}),
    ));
    let catalog = Arc::clone(engine.catalog());
    let mut cached = CachedScreen::default();
    let view = ViewState::default();
    for (now, omit) in [(0.0, false), (0.5, true), (2.0, true), (3.0, false)] {
        paint(&mut engine, now, |engine, frame| {
            let px = frame.metrics.scale.get() * ui::FONT_DESIGN_PIXEL_TEXELS as f32;
            engine.draw(
                ScreenArt {
                    omit_controls: if omit { &["root"] } else { &[] },
                    ..ScreenArt::default()
                },
                frame,
                |env, root| {
                    cached.render_with(
                        "fixture.root",
                        &catalog,
                        &Context::desktop(),
                        DataSource::new(),
                        (root, px, [0; 3]),
                        env,
                        &view,
                    )
                },
            )
        });
        assert!(engine.animator().take_events().is_empty());
    }
}

#[test]
fn generic_default_state_without_a_paint_view_remains_visible() {
    let mut engine = engine(state_catalog());
    let catalog = Arc::clone(engine.catalog());
    let mut cached = CachedScreen::default();
    let (_, nodes) = paint(&mut engine, 0.0, |engine, frame| {
        let px = frame.metrics.scale.get() * ui::FONT_DESIGN_PIXEL_TEXELS as f32;
        engine.draw(ScreenArt::default(), frame, |env, root| {
            cached.render_with(
                "fixture.root",
                &catalog,
                &Context::desktop(),
                DataSource::new(),
                (root, px, [0; 3]),
                env,
                &ViewState::default(),
            )
        })
    });
    assert_eq!(
        nodes
            .iter()
            .filter(|node| matches!(node.visual(), UiVisual::Solid { .. }))
            .count(),
        1
    );
}

#[test]
fn server_form_entry_keeps_actual_hover_and_press_hits() {
    let mut engine = engine(state_catalog());
    let key = format!(
        "/{}",
        json_ui::form_template(&model()).split_once('.').unwrap().1
    );
    for (index, selected) in ["idle_hit", "hover_hit", "press_hit"]
        .into_iter()
        .enumerate()
    {
        let view = ViewState {
            hovered: (index == 1).then(|| key.clone()),
            pressed: (index == 2).then(|| key.clone()),
            ..ViewState::default()
        };
        let (frame, _) = paint(&mut engine, index as f64, |engine, frame| {
            engine.render(&model(), &view, frame)
        });
        assert_state_hits(&frame, selected);
    }
}

#[test]
fn server_form_mixed_sprite_programs_wait_for_artwork() {
    for kind in ["flip_book", "aseprite_flip_book"] {
        let mut engine = engine(catalog(
            json!({
                "type":"panel","size":[100,50],"offset":"@fixture.still","controls":[
                    {"strip":{"type":"image","size":[40,20],"texture":"textures/fixture_strip","uv_size":[8,8],"anims":["@fixture.move","@fixture.book"]}}
                ]
            }),
            json!({
                "still":{"anim_type":"offset","from":[0,0],"to":[0,0],"duration":1},
                "move":{"anim_type":"offset","from":[0,0],"to":[20,0],"duration":1,"end_event":"fixture.unseen"},
                "book":{"anim_type":kind,"fps":4,"frame_count":4,"wait_until_rendered_to_play":true}
            }),
        ));
        for now in [0.0, 2.0] {
            let (_, nodes) = paint(&mut engine, now, |engine, frame| {
                engine.render(&model(), &ViewState::default(), frame)
            });
            assert!(nodes.is_empty());
            assert!(
                engine.animator().take_events().is_empty(),
                "{kind} must not advance before artwork is ready"
            );
        }
        engine
            .textures
            .set_full_res(std::collections::HashMap::from([(
                "textures/fixture_strip".into(),
                IconRef {
                    page: 7,
                    uv: [0, 0, 32, 8],
                    glint: false,
                },
            )]));
        let (_, nodes) = paint(&mut engine, 3.0, |engine, frame| {
            engine.render(&model(), &ViewState::default(), frame)
        });
        assert!(!nodes.is_empty(), "ready artwork paints");
        assert!(
            engine.animator().take_events().is_empty(),
            "first ready frame starts the mixed component"
        );
    }
}

#[test]
fn server_form_entry_moves_paint_and_input_together() {
    let mut engine = engine(catalog(
        json!({
            "type":"panel","size":[200,100],"clips_children":true,"controls":[
                {"page":{"type":"panel","size":[40,20],"offset":"@fixture.enter","anchor_from":"top_left","anchor_to":"top_left","controls":[
                    {"press":{"type":"button","controls":[{"fill":{"type":"image","color":[1,0,0]}}]}}
                ]}}
            ]
        }),
        json!({"enter":{"anim_type":"offset","from":[0,100],"to":[20,10],"duration":1,"end_event":"fixture.arrived"}}),
    ));
    let mut previous = None;
    for now in [0.0, 1.0, 2.0] {
        let (frame, nodes) = paint(&mut engine, now, |engine, frame| {
            engine.render(&model(), &ViewState::default(), frame)
        });
        if now == 0.0 {
            assert!(!frame.hits.iter().any(|hit| hit.contains([25.0, 15.0])));
        } else {
            let press = frame.hits.iter().find(|hit| hit.name == "press").unwrap();
            assert_eq!(
                [press.rect.x, press.rect.y, press.rect.w, press.rect.h],
                [20.0, 10.0, 40.0, 20.0]
            );
            assert!(press.contains([25.0, 15.0]));
            assert!(!press.contains([5.0, 105.0]));
            assert!(
                nodes
                    .iter()
                    .any(|node| matches!(node.visual(), UiVisual::Solid { .. }))
            );
            if let Some(previous) = &previous {
                assert!(Arc::ptr_eq(previous, &frame.hits));
            }
            previous = Some(Arc::clone(&frame.hits));
        }
    }
    assert_eq!(engine.animator().take_events().len(), 1);
}

#[test]
fn server_form_chained_growth_and_parent_percentage_slide_remain_usable() {
    let mut engine = engine(catalog(
        json!({
            "type":"panel","size":[200,100],"clips_children":true,"controls":[
                {"pages":{"type":"panel","size":[200,100],"offset":"@fixture.hold","anchor_from":"top_left","anchor_to":"top_left","controls":[
                    {"logo":{"type":"image","size":"@fixture.start","color":[1,0,0]}},
                    {"menu":{"type":"panel","size":[200,100],"offset":[0,"100%"],"anchor_from":"top_left","anchor_to":"top_left","controls":[
                        {"press":{"type":"button","size":[40,20],"anchor_from":"top_left","anchor_to":"top_left","controls":[{"fill":{"type":"image","color":[0,1,0]}}]}}
                    ]}}
                ]}}
            ]
        }),
        json!({
            "start":{"anim_type":"size","from":[0,0],"to":[0,0],"duration":0.25,"next":"@fixture.grow"},
            "grow":{"anim_type":"size","from":[0,0],"to":["70%","70%"],"duration":0.75},
            "hold":{"anim_type":"offset","from":[0,0],"to":[0,0],"duration":1,"next":"@fixture.slide"},
            "slide":{"anim_type":"offset","from":[0,0],"to":[0,"-100%"],"duration":1}
        }),
    ));
    for now in [0.0, 1.0, 2.0] {
        let (frame, nodes) = paint(&mut engine, now, |engine, frame| {
            engine.render(&model(), &ViewState::default(), frame)
        });
        let active = frame
            .hits
            .iter()
            .any(|hit| hit.name == "press" && hit.contains([5.0, 5.0]));
        assert_eq!(active, now == 2.0);
        if now == 1.0 {
            assert!(
                nodes.iter().any(|node| matches!(
                    node.visual(),
                    UiVisual::Solid {
                        color: [255, 0, 0, 255],
                        ..
                    }
                )),
                "zero-sized logo grows before the slide"
            );
        }
        if now == 2.0 {
            assert!(
                nodes.iter().any(|node| matches!(
                    node.visual(),
                    UiVisual::Solid {
                        color: [0, 255, 0, 255],
                        ..
                    }
                )),
                "the entered menu paints"
            );
        }
    }
}

#[test]
fn server_form_destruction_hides_static_children_without_restarting() {
    let name = json_ui::form_template(&model()).split_once('.').unwrap().1;
    let mut engine = engine(catalog(
        json!({
            "type":"panel","size":[100,50],"offset":"@fixture.leave","controls":[
                {"fill":{"type":"image","size":[20,10],"color":[1,0,0]}}
            ]
        }),
        json!({"leave":{"anim_type":"offset","from":[0,0],"to":[20,0],"duration":1,"destroy_at_end":name}}),
    ));
    for now in [0.0, 1.0, 2.0, 3.0] {
        let (_, nodes) = paint(&mut engine, now, |engine, frame| {
            engine.render(&model(), &ViewState::default(), frame)
        });
        assert_eq!(nodes.is_empty(), now >= 1.0);
    }
    assert_eq!(engine.animator().take_events().len(), 1);
}
