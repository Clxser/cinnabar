//! Incremental binds rebuild only the subtrees that read a changed key.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::{BindState, DataSource, EmptyLibrary, ResolvedControl, Scalar};

/// Counts the expanded tree, including controls materialized by nested factories.
fn budget_nodes(tree: &ResolvedControl) -> usize {
    1 + tree.children.iter().map(budget_nodes).sum::<usize>()
}

/// A stable catalog with factory references whose feeds can exceed a caller's budget.
fn budget_catalog() -> crate::Catalog {
    let mut catalog = crate::Catalog::default();
    catalog.overlay_text("ui/budget.json", r#"{"namespace":"budget",
        "leaf":{"type":"panel"},
        "row":{"type":"panel","factory":{"name":"leaves","control_ids":{"leaf":"budget.leaf"}}},
        "nested":{"type":"panel","factory":{"name":"rows","control_ids":{"row":"budget.row"}}},
        "reuse":{"type":"panel","controls":[
            {"expanding":{"type":"panel","factory":{"name":"rows","control_ids":{"leaf":"budget.leaf"}}}},
            {"stable":{"type":"panel","controls":[{"a":{}},{"b":{}},{"c":{}},{"d":{}},{"e":{}},{"f":{}},{"g":{}},{"h":{}}]}}
        ]}}"#);
    catalog
}

#[test]
fn a_caller_budget_limits_nested_factory_materialization_on_each_bind() {
    let catalog = budget_catalog();
    let context = crate::Context::default();
    let root = Arc::new(
        crate::resolve(&catalog, "budget.nested", &context)
            .control
            .unwrap(),
    );
    let library = crate::CatalogLibrary {
        catalog: &catalog,
        context: &context,
    };
    let mut state = BindState::new().with_node_limit(17);
    assert_eq!(BindState::default().node_limit(), crate::resolve::MAX_NODES);
    assert_eq!(BindState::new().with_node_limit(0).node_limit(), 1);
    assert_eq!(
        BindState::new().with_node_limit(usize::MAX).node_limit(),
        crate::resolve::MAX_NODES
    );
    for revision in 0..3 {
        let mut data = DataSource::new();
        data.set_factory(
            "rows",
            (0..100)
                .map(|_| crate::FactoryItem::new("row", 0.0))
                .collect(),
        );
        data.set_factory(
            "leaves",
            (0..100)
                .map(|_| crate::FactoryItem::new("leaf", 0.0))
                .collect(),
        );
        data.set_global("#unread", Scalar::Num(f64::from(revision)));
        let bound = crate::bind_incremental(&root, &Arc::new(data), &library, &mut state);
        assert_eq!(budget_nodes(&bound), 17);
        assert!(state.node_budget_exceeded);
        assert!(
            state.rebuilt() <= 17,
            "materialized more controls than the cap"
        );
    }
}

#[test]
fn reused_subtrees_share_the_creation_budget_and_recover_after_pressure() {
    let catalog = budget_catalog();
    let context = crate::Context::default();
    let root = Arc::new(
        crate::resolve(&catalog, "budget.reuse", &context)
            .control
            .unwrap(),
    );
    let library = crate::CatalogLibrary {
        catalog: &catalog,
        context: &context,
    };
    let mut state = BindState::new().with_node_limit(17);
    let mut bound = None;
    let mut measures = crate::MeasureCache::default();
    for (rows, expected) in [(0, 11), (10, 17), (0, 11), (10, 17)] {
        let mut data = DataSource::new();
        data.set_factory(
            "rows",
            (0..rows)
                .map(|_| crate::FactoryItem::new("leaf", 0.0))
                .collect(),
        );
        let data = Arc::new(data);
        if let Some(bound) = bound.as_mut() {
            crate::rebind(&root, &data, &library, &mut state, bound, &mut measures);
        } else {
            bound = Some(crate::bind_incremental(&root, &data, &library, &mut state));
        }
        assert_eq!(budget_nodes(bound.as_ref().unwrap()), expected);
        assert!(state.tree.as_ref().unwrap().track.count <= 17);
    }
}

fn control(
    name: &str,
    kind: &str,
    properties: Value,
    children: Vec<ResolvedControl>,
) -> ResolvedControl {
    let Value::Object(properties) = properties else {
        unreachable!("control properties are an object");
    };
    ResolvedControl {
        name: name.into(),
        control_type: Some(kind.into()),
        base: None,
        unresolved_base: None,
        properties: properties
            .into_iter()
            .collect::<std::collections::BTreeMap<_, _>>()
            .into(),
        children,
        factory: None,
    }
}

/// A label showing the screen global `name`.
fn reader(control_name: &str, name: &str) -> ResolvedControl {
    control(
        control_name,
        "label",
        json!({ "text": name, "bindings": [{ "binding_name": name }] }),
        Vec::new(),
    )
}

#[test]
fn an_unread_change_leaves_a_subtree_reading_many_keys_standing() {
    // Enough distinct reads that a fixed-size summary of them would claim every key.
    const READERS: usize = 2_000;
    let wide = control(
        "wide",
        "panel",
        json!({}),
        (0..READERS)
            .map(|index| reader(&format!("reader{index}"), &format!("#value{index}")))
            .collect(),
    );
    let root = Arc::new(control(
        "root",
        "panel",
        json!({}),
        vec![wide, reader("narrow", "#other")],
    ));
    let data = |other: &str| {
        let mut data = DataSource::new();
        for index in 0..READERS {
            data.set_global(format!("#value{index}"), Scalar::Text("shown".into()));
        }
        data.set_global("#other", Scalar::Text(other.into()));
        Arc::new(data)
    };
    let mut state = BindState::new();
    // The second bind settles what the first created.
    for other in ["first", "first", "second"] {
        crate::bind_incremental(&root, &data(other), &EmptyLibrary, &mut state);
    }
    let rebuilt = state.rebuilt_paths();
    assert!(
        rebuilt.iter().any(|path| path == "/root/narrow"),
        "{rebuilt:?}"
    );
    assert!(
        !rebuilt.iter().any(|path| path.starts_with("/root/wide")),
        "{rebuilt:?}"
    );
}
