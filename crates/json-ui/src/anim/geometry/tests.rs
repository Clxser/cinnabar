use super::*;

struct Metrics;
impl crate::TextMeasure for Metrics {
    /// The invalidation fixture has no text.
    fn extent(&self, _: &str) -> [f64; 2] {
        [0.0; 2]
    }
}
impl crate::TextureSource for Metrics {
    /// The invalidation fixture contains only panels.
    fn texture(&self, _: &str) -> Option<crate::TextureMeta> {
        None
    }
}

#[test]
fn geometry_changes_scan_measurements_once_per_frame() {
    let controls: Vec<_> = (0..64)
        .map(|index| json!({format!("item{index}"):{"type":"panel","size":[10,10],"offset":"@fixture.move"}}))
        .collect();
    let mut catalog = crate::Catalog::default();
    catalog.overlay_text(
        "ui/fixture.json",
        &json!({
            "namespace":"fixture",
            "move":{"anim_type":"offset","from":[0,0],"to":[10,10],"duration":1},
            "root":{"type":"panel","size":[200,100],"controls":controls}
        })
        .to_string(),
    );
    let bound = crate::resolve(&catalog, "fixture.root", &crate::Context::desktop())
        .control
        .unwrap();
    let env = LayoutEnv {
        text: &Metrics,
        textures: &Metrics,
    };
    let view = ViewState::default();
    let mut layout = GeometryLayout::new(&bound, [200.0, 100.0], &env, &view).unwrap();
    let mut animator = Animator::new();
    drop(layout.sample(&mut animator, 0.0, None, &env, &view, |_| true));
    let entries = layout.measures.length_entries();
    let before = layout.measures.length_entries_scanned;
    drop(layout.sample(&mut animator, 0.5, None, &env, &view, |_| true));
    let visited = layout.measures.length_entries_scanned - before;
    assert!(entries > 0);
    assert!(
        visited <= entries,
        "visited {visited} retained entries from a {entries}-entry cache"
    );
}
