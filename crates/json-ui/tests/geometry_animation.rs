use std::sync::Arc;

use json_ui::{
    Animator, Catalog, Context, DrawNode, FormRender, GeometryLayout, HitRegion, LayoutEnv,
    TextMeasure, TextureMeta, TextureSource, ViewState,
};
use serde_json::{Value, json};

struct Metrics;
impl TextMeasure for Metrics {
    /// Gives original labels deterministic dimensions without installed carriers.
    fn extent(&self, text: &str) -> [f64; 2] {
        [text.chars().count() as f64 * 6.0, 10.0]
    }
}
impl TextureSource for Metrics {
    /// Uses a small synthetic texture so sprite tessellation requires no files.
    fn texture(&self, _: &str) -> Option<TextureMeta> {
        Some(TextureMeta::plain([16.0, 16.0]))
    }
}
/// Borrows the deterministic measurement sources shared by the original fixtures.
fn env() -> LayoutEnv<'static> {
    LayoutEnv {
        text: &Metrics,
        textures: &Metrics,
    }
}

struct Screen {
    layout: GeometryLayout,
    animator: Animator,
}
impl Screen {
    /// Resolves a tiny original screen with optional client-owned paint transforms.
    fn new(mut document: Value) -> Self {
        document["namespace"] = json!("fixture");
        let mut catalog = Catalog::default().with_paint_transforms();
        catalog.overlay_text("ui/fixture.json", &document.to_string());
        let bound = json_ui::resolve(&catalog, "fixture.root", &Context::desktop())
            .control
            .unwrap();
        Self {
            layout: GeometryLayout::new(&bound, [200.0, 100.0], &env(), &ViewState::default())
                .unwrap(),
            animator: Animator::new(),
        }
    }
    /// Runs the same sample-before-layout and paint order as the engine at supplied time.
    fn frame(&mut self, now: f64, view: &ViewState) -> Arc<FormRender> {
        let frame = self
            .layout
            .sample(&mut self.animator, now, None, &env(), view);
        for node in frame.nodes.iter() {
            node.animate(&mut self.animator, now, None, Some(&Metrics));
        }
        self.animator.end_frame();
        frame
    }
}
/// Returns a drawn primitive by its original fixture name.
fn draw<'a>(frame: &'a FormRender, name: &str) -> &'a DrawNode {
    frame
        .nodes
        .iter()
        .find(|node| node.name == name)
        .unwrap_or_else(|| panic!("missing draw {name}"))
}
/// Returns an interactive control by its original fixture name.
fn hit<'a>(frame: &'a FormRender, name: &str) -> &'a HitRegion {
    frame
        .hits
        .iter()
        .find(|node| node.name == name)
        .unwrap_or_else(|| panic!("missing hit {name}"))
}

#[test]
fn zero_sized_growth_uses_current_size_and_anchor() {
    for from in [[0, 0], [0, 10], [10, 0]] {
        for (anchor, expected) in [("center", [80.0, 40.0]), ("bottom_right", [160.0, 80.0])] {
            let mut screen = Screen::new(json!({
                "grow":{"anim_type":"size","from":from,"to":[40,20],"duration":1},
                "root":{"type":"image","texture":"textures/fixture","keep_ratio":false,"size":"@fixture.grow","anchor_from":anchor,"anchor_to":anchor}
            }));
            screen.frame(0.0, &ViewState::default());
            let frame = screen.frame(1.0, &ViewState::default());
            let rect = draw(&frame, "root").dest;
            assert_eq!(
                [rect.x, rect.y, rect.w, rect.h],
                [expected[0], expected[1], 40.0, 20.0]
            );
        }
    }
}

#[test]
fn parent_growth_reflows_fixed_and_percentage_children() {
    let mut screen = Screen::new(json!({
        "grow":{"anim_type":"size","from":[0,0],"to":[200,100],"duration":1},
        "root":{"type":"panel","size":"@fixture.grow","anchor_from":"top_left","anchor_to":"top_left","controls":[
            {"fixed":{"type":"button","size":[20,10]}},
            {"relative":{"type":"image","texture":"textures/fixture","keep_ratio":false,"size":["50%","50%"]}}
        ]}
    }));
    screen.frame(0.0, &ViewState::default());
    let frame = screen.frame(1.0, &ViewState::default());
    let fixed = hit(&frame, "fixed").rect;
    assert_eq!(
        [fixed.x, fixed.y, fixed.w, fixed.h],
        [90.0, 45.0, 20.0, 10.0]
    );
    let relative = draw(&frame, "relative").dest;
    assert_eq!(
        [relative.x, relative.y, relative.w, relative.h],
        [50.0, 25.0, 100.0, 50.0]
    );
}

#[test]
fn mixed_units_follow_a_parent_resizing_at_the_same_clock() {
    let mut screen = Screen::new(json!({
        "grow":{"anim_type":"size","from":[100,50],"to":[200,100],"duration":1},
        "move":{"anim_type":"offset","from":[10,0],"to":["50%","-100%"],"duration":1},
        "root":{"type":"panel","size":"@fixture.grow","anchor_from":"top_left","anchor_to":"top_left","controls":[
            {"press":{"type":"button","size":[10,10],"offset":"@fixture.move","anchor_from":"top_left","anchor_to":"top_left"}}
        ]}
    }));
    screen.frame(0.0, &ViewState::default());
    let frame = screen.frame(0.5, &ViewState::default());
    let rect = hit(&frame, "press").rect;
    assert_eq!([rect.x, rect.y, rect.w, rect.h], [42.5, -37.5, 10.0, 10.0]);
}

#[test]
fn entering_page_rebuilds_nested_clips_and_input_geometry() {
    let mut screen = Screen::new(json!({
        "move":{"anim_type":"offset","from":[0,100],"to":[0,0],"duration":1},
        "root":{"type":"panel","size":[200,100],"clips_children":true,"clip_offset":[10,10],"controls":[
            {"page":{"type":"panel","size":[200,100],"offset":"@fixture.move","anchor_from":"top_left","anchor_to":"top_left","clips_children":true,"clip_offset":[20,5],"controls":[
                {"middle":{"type":"panel","controls":[
                    {"press":{"type":"button","size":["100%","100%"],"button_mappings":[{"from_button_id":"button.menu_select","to_button_id":"fixture.choose","mapping_type":"pressed"}],"controls":[
                        {"fill":{"type":"image","texture":"textures/fixture","keep_ratio":false,"size":["100%","100%"]}}
                    ]}}
                ]}}
            ]}}
        ]}
    }));
    let initial = screen.frame(0.0, &ViewState::default());
    assert!(initial.nodes.is_empty());
    let middle = screen.frame(0.5, &ViewState::default());
    assert_eq!(draw(&middle, "fill").clip.h, 35.0);
    let frame = screen.frame(1.0, &ViewState::default());
    let fill = draw(&frame, "fill");
    assert_eq!(
        [fill.clip.x, fill.clip.y, fill.clip.w, fill.clip.h],
        [20.0, 10.0, 160.0, 80.0]
    );
    assert_eq!(hit(&frame, "press").clip, fill.clip);
    assert!(initial.nodes.is_empty(), "earlier snapshots stay immutable");
    assert_eq!(draw(&middle, "fill").clip.h, 35.0);
    assert!(json_ui::hit_test(&frame.hits, [30.0, 20.0]).is_some());
    assert!(json_ui::hit_test(&frame.hits, [30.0, 120.0]).is_none());
    assert!(json_ui::hit_test(&frame.hits, [15.0, 20.0]).is_none());
}

#[test]
fn paint_transform_applies_once_after_sampled_geometry_and_input() {
    let mut screen = Screen::new(json!({
        "move":{"anim_type":"offset","from":[0,0],"to":[20,10],"duration":1},
        "root":{"type":"panel","size":[40,20],"offset":"@fixture.move","anchor_from":"top_left","anchor_to":"top_left","cinnabar_transform":{"scale":2,"origin":[0,0]},"controls":[
            {"press":{"type":"button","size":[20,10],"anchor_from":"top_left","anchor_to":"top_left","controls":[
                {"fill":{"type":"image","texture":"textures/fixture","keep_ratio":false}}
            ]}}
        ]}
    }));
    screen.frame(0.0, &ViewState::default());
    let frame = screen.frame(1.0, &ViewState::default());
    let fill = draw(&frame, "fill");
    assert_eq!([fill.dest.x, fill.dest.y], [20.0, 10.0]);
    assert_eq!(
        fill.anim.as_ref().unwrap().transform,
        hit(&frame, "press").transform
    );
    assert!(hit(&frame, "press").contains([55.0, 25.0]));
    assert!(!hit(&frame, "press").contains([65.0, 25.0]));
}

#[test]
fn geometry_refresh_preserves_scroll_culling_and_settled_frame_reuse() {
    let rows:Vec<_>=(0..20).map(|index|json!({format!("row{index}"):{"type":"button","size":[100,10],"controls":[{"label":{"type":"label","text":format!("Row {index}"),"size":[100,10]}}]}})).collect();
    let mut screen = Screen::new(json!({
        "move":{"anim_type":"offset","from":[0,100],"to":[0,0],"duration":1,"end_event":"fixture.arrived"},
        "root":{"type":"scroll_view","size":[100,50],"offset":"@fixture.move","anchor_from":"top_left","anchor_to":"top_left","scroll_view_port":"port","scroll_content":"content","scrollbar_track":"track","scrollbar_box":"box","controls":[
            {"port":{"type":"panel","size":[100,50],"clips_children":true,"anchor_from":"top_left","anchor_to":"top_left","controls":[
                {"content":{"type":"stack_panel","size":[100,"100%c"],"anchor_from":"top_left","anchor_to":"top_left","controls":rows}}
            ]}},
            {"track":{"type":"scroll_track","size":[4,50],"anchor_from":"top_right","anchor_to":"top_right","controls":[
                {"box":{"type":"scrollbar_box","size":[4,50],"draggable":"vertical"}}
            ]}}
        ]}
    }));
    screen.frame(0.0, &ViewState::default());
    let arrived = screen.frame(1.0, &ViewState::default());
    assert_eq!(screen.animator.take_events().len(), 1);
    assert!(json_ui::hit_test(&arrived.hits, [5.0, 5.0]).is_some());
    assert!(arrived.nodes.len() < 20, "offscreen rows stay culled");
    let mut view = ViewState::default();
    view.scroll.insert("/root".into(), 100.0);
    let scrolled = screen.frame(1.5, &view);
    assert!(!hit(&scrolled, "row0").contains([5.0, 5.0]));
    assert!(hit(&scrolled, "row10").contains([5.0, 5.0]));
    assert!(
        !scrolled
            .nodes
            .iter()
            .any(|node| matches!(&node.draw, json_ui::Draw::Text { text, .. } if text == "Row 0"))
    );
    let settled = screen.frame(2.0, &view);
    assert!(
        Arc::ptr_eq(&scrolled, &settled),
        "unchanged geometry reuses its frame"
    );
    assert!(
        screen.animator.take_events().is_empty(),
        "re-layout must not restart animation events"
    );
}
