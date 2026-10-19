use super::*;

struct Metrics;
impl crate::TextMeasure for Metrics {
    /// The geometry cache fixture has no text.
    fn extent(&self, _: &str) -> [f64; 2] {
        [0.0; 2]
    }
}
impl crate::TextureSource for Metrics {
    /// The geometry cache fixture contains only panels.
    fn texture(&self, _: &str) -> Option<crate::TextureMeta> {
        None
    }
}

#[test]
fn fractional_parent_sizes_retain_only_current_descendant_measurements() {
    let mut catalog = crate::Catalog::default();
    catalog.overlay_text(
        "ui/fixture.json",
        r#"{
        "namespace":"fixture",
        "grow":{"anim_type":"size","from":[100,50],"to":[200,100],"duration":1},
        "root":{"type":"panel","size":[100,50],"anims":["@fixture.grow"],"controls":[
            {"fixed":{"type":"panel","size":["100%","100%"],"controls":[
                {"inner":{"type":"panel","size":[10,10]}}
            ]}}
        ]}
    }"#,
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
    let mut animator = Animator::starting_at(0.0);
    for step in 0..128 {
        drop(layout.sample(&mut animator, f64::from(step) / 256.0, None, &env, &view));
        animator.end_frame();
    }
    let retained = layout.measures.retained_entries();
    let mut fresh = GeometryLayout::new(&bound, [200.0, 100.0], &env, &view).unwrap();
    drop(fresh.sample(
        &mut Animator::starting_at(0.0),
        127.0 / 256.0,
        None,
        &env,
        &view,
    ));
    let current = fresh.measures.retained_entries();
    assert!(current > 0);
    assert!(
        retained <= current,
        "retained {retained} memo entries for a {current}-entry current layout"
    );
}

#[test]
fn mixed_components_keep_their_original_paint_activation() {
    for kind in [
        "alpha",
        "color",
        "uv",
        "clip",
        "flip_book",
        "aseprite_flip_book",
        "wait",
    ] {
        let mut catalog = crate::Catalog::default();
        catalog.overlay_text("ui/fixture.json",&json!({
            "namespace":"fixture",
            "still":{"anim_type":"offset","from":[0,0],"to":[0,0],"duration":10},
            "move":{"anim_type":"offset","from":[0,0],"to":[20,0],"duration":1,"end_event":"fixture.unseen"},
            "other":{"anim_type":kind,"from":0,"to":1,"duration":1},
            "root":{"type":"panel","size":[200,100],"anchor_from":"top_left","anchor_to":"top_left","offset":"@fixture.still","controls":[
                {"mixed":{"type":"image","size":[10,10],"color":[1,0,0],"anchor_from":"top_left","anchor_to":"top_left","anims":["@fixture.move","@fixture.other"]}}
            ]}
        }).to_string());
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
        drop(layout.sample(&mut animator, 0.0, None, &env, &view));
        animator.end_frame();
        let frame = layout.sample(&mut animator, 2.0, None, &env, &view);
        assert!(
            animator.take_events().is_empty(),
            "{kind} must not start in layout"
        );
        let mixed = frame
            .nodes
            .iter()
            .find(|node| node.name == "mixed")
            .unwrap();
        assert_eq!(mixed.dest.x, 0.0);
        let drawn = mixed.animate(&mut animator, 2.0, None, None);
        assert_eq!(
            drawn.dest.x, 0.0,
            "{kind} component starts with its first paint"
        );
        assert!(animator.take_events().is_empty());
        let mut only_mixed = bound.children[0].clone();
        only_mixed.name = "root".into();
        assert!(GeometryLayout::new(&only_mixed, [200.0, 100.0], &env, &view).is_none());
    }
}
