use std::sync::{Arc, OnceLock};

use json_ui::{Catalog, Context, LayoutEnv, TextMeasure, TextureMeta, TextureSource, ViewState};
use ui::{DpiScale, SafeArea, TextLayoutCache, UiVisual};

use crate::{
    engine::{
        FormEngine, ScreenArt,
        screen::{PaintFrame, ScreenRegistry},
    },
    sources::{ImageWork, RemoteImage, RemoteSource},
    test_support::fixture_font,
    text_metrics::TextMetrics,
};

struct Offline;
impl RemoteSource for Offline {
    /// The original geometry fixture has no external images.
    fn state(&self, _: &str) -> RemoteImage {
        RemoteImage::Failed
    }
}
struct Metrics;
impl TextMeasure for Metrics {
    /// No text participates in the original solid-color fixture.
    fn extent(&self, _: &str) -> [f64; 2] {
        [0.0; 2]
    }
}
impl TextureSource for Metrics {
    /// Solid-color images require no texture metadata.
    fn texture(&self, _: &str) -> Option<TextureMeta> {
        None
    }
}

#[test]
fn engine_publishes_sampled_geometry_for_paint_and_input() {
    static REGISTRY: OnceLock<ScreenRegistry> = OnceLock::new();
    let bytes = assets::encode_ui_catalog([1; 32], &[], &[], &[], &[]).unwrap();
    let engine = FormEngine::new(
        Arc::new(assets::RuntimeUiAssets::decode(&bytes).unwrap()),
        Catalog::default(),
        0,
        REGISTRY.get_or_init(|| ScreenRegistry::new(Context::desktop())),
        Arc::new(Offline),
        ImageWork::inline(),
    );
    let mut catalog = Catalog::default();
    catalog.overlay_text("ui/fixture.json",r#"{
        "namespace":"fixture",
        "enter":{"anim_type":"offset","from":[0,100],"to":[20,10],"duration":1,"end_event":"fixture.arrived"},
        "root":{"type":"panel","size":[200,100],"clips_children":true,"controls":[
            {"page":{"type":"panel","size":[40,20],"offset":"@fixture.enter","anchor_from":"top_left","anchor_to":"top_left","controls":[
                {"press":{"type":"button","controls":[{"fill":{"type":"image","color":[1,0,0]}}]}}
            ]}}
        ]}
    }"#);
    let bound = json_ui::resolve(&catalog, "fixture.root", &Context::desktop())
        .control
        .unwrap();
    let source = json_ui::render_bound(
        bound,
        [200.0, 100.0],
        &LayoutEnv {
            text: &Metrics,
            textures: &Metrics,
        },
        &ViewState::default(),
    );
    let font = fixture_font();
    let metrics = TextMetrics::for_viewport([400, 200], DpiScale::new(1.0).unwrap(), None);
    let px = metrics.scale.get() * ui::FONT_DESIGN_PIXEL_TEXELS as f32;
    let mut layouts = TextLayoutCache::new(16, 1024 * 1024);
    let mut previous = None;
    for now in [0.0, 1.0, 2.0] {
        let mut nodes = Vec::new();
        let mut next = 1;
        let frame = engine
            .draw(
                ScreenArt::default(),
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
                |_, _| Some(&source),
            )
            .unwrap()
            .unwrap();
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
                    .any(|node| matches!(node.visual(), UiVisual::Solid { .. })),
                "entering geometry paints"
            );
            if let Some(previous) = &previous {
                assert!(Arc::ptr_eq(previous, &frame.hits));
            }
            previous = Some(Arc::clone(&frame.hits));
        }
        engine.animator().end_frame();
    }
    assert_eq!(engine.animator().take_events().len(), 1);
}
