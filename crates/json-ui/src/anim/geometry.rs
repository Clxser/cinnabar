//! Size and offset writes are layout inputs: measure, clipping, culling and input must
//! observe them before emission. Paint still samples the other animation channels.

use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use serde_json::{Value, json};

use super::{Animator, ControlAnims, GEOMETRY_KEY, GRAPH_KEY};
use crate::{FormRender, LaidOut, LayoutEnv, MeasureCache, ResolvedControl, ViewState};

struct Program {
    path: Vec<usize>,
    animation: Arc<ControlAnims>,
}

/// A bound screen's geometry animation programs and its latest measured frame.
/// Recreate it when the bound screen, viewport or measurement environment changes.
pub struct GeometryLayout {
    programs: Vec<Program>,
    root: [f64; 2],
    view: ViewState,
    measures: MeasureCache,
    rendered: Option<Arc<FormRender>>,
}

impl GeometryLayout {
    /// Captures geometry programs before culling can omit an entering control. Screens
    /// with no size or offset animation keep their existing retained rendering path.
    pub fn new(
        bound: &ResolvedControl,
        root: [f64; 2],
        env: &LayoutEnv,
        view: &ViewState,
    ) -> Option<Self> {
        if !has_geometry(bound) {
            return None;
        }
        let mut paths = HashMap::new();
        control_paths(bound, &mut Vec::new(), &mut paths);
        let (laid, _) = crate::layout_with(bound, root, env, view);
        let mut programs = Vec::new();
        collect(&laid, &paths, &mut programs);
        if programs.is_empty() {
            return None;
        }
        let mut bound = bound.clone();
        for program in &programs {
            at(&mut bound, &program.path)
                .properties
                .insert(GEOMETRY_KEY.into(), Value::Bool(true));
        }
        let mut measures = MeasureCache::default();
        let rendered = crate::render_bound_gated(bound, root, env, view, &mut measures);
        Some(Self {
            programs,
            root,
            view: view.layout_part(),
            measures,
            rendered: Some(Arc::new(rendered)),
        })
    }

    /// Samples caller-supplied time before measuring changed geometry. Unchanged writes
    /// reuse the complete frame, including its clips, scroll extents and hit regions.
    pub fn sample(
        &mut self,
        animator: &mut Animator,
        now: f64,
        clocks: Option<&BTreeMap<String, f64>>,
        env: &LayoutEnv,
        view: &ViewState,
    ) -> Arc<FormRender> {
        let mut writes = Vec::with_capacity(self.programs.len());
        for program in &self.programs {
            let (written, samples) = animator.sample_geometry(&program.animation, now, clocks);
            writes.push(
                [
                    ("size", written.size, samples.size),
                    ("offset", written.offset, samples.offset),
                ]
                .into_iter()
                .filter_map(|(name, pixels, sample)| {
                    let pixels = pixels?;
                    let value = sample.map_or_else(
                        || json!(pixels),
                        |sample| {
                            layout_value(
                                &program.animation.graph.nodes[sample.node],
                                sample.progress,
                                pixels,
                            )
                        },
                    );
                    Some((name, value))
                })
                .collect::<Vec<_>>(),
            );
        }
        let saved = self.rendered.take().expect("geometry frame retained");
        let changed = self.programs.iter().zip(&writes).any(|(program, write)| {
            let properties = &at_ref(&saved.bound, &program.path).properties;
            write
                .iter()
                .any(|(name, value)| properties.get(*name) != Some(value))
        });
        let rendered = if changed || !self.view.same_layout(view) {
            let mut saved = match Arc::try_unwrap(saved) {
                Ok(saved) => saved,
                Err(saved) => {
                    // A retained snapshot requires a new tree with new control addresses.
                    self.measures = MeasureCache::default();
                    (*saved).clone()
                }
            };
            for (program, write) in self.programs.iter().zip(writes) {
                self.measures
                    .update_properties(&mut saved.bound, &program.path, |properties| {
                        let mut changed = false;
                        for (name, value) in write {
                            if properties.get(name) != Some(&value) {
                                properties.insert(name.into(), value);
                                changed = true;
                            }
                        }
                        changed
                    });
            }
            self.view = view.layout_part();
            Arc::new(crate::render_bound_gated(
                saved.bound,
                self.root,
                env,
                view,
                &mut self.measures,
            ))
        } else {
            saved
        };
        self.rendered = Some(Arc::clone(&rendered));
        rendered
    }
}

/// Interpolates authored coefficients so percentages keep following current layout dimensions.
fn layout_value(node: &super::AnimNode, progress: f32, pixels: [f32; 2]) -> Value {
    Value::Array(
        (0..2)
            .map(|axis| {
                node.from_expr
                    .get(axis)
                    .zip(node.to_expr.get(axis))
                    .and_then(|(from, to)| {
                        crate::length_from_value(from).blend(
                            &crate::length_from_value(to),
                            |from, to| {
                                f64::from(node.easing.apply(from as f32, to as f32, progress))
                            },
                        )
                    })
                    .map_or_else(|| json!(pixels[axis]), |length| length.to_value())
            })
            .collect(),
    )
}

/// Whether this bound subtree contains a linked size or offset program.
fn has_geometry(control: &ResolvedControl) -> bool {
    control
        .properties
        .get(GRAPH_KEY)
        .and_then(|value| serde_json::from_value::<super::AnimGraph>(value.clone()).ok())
        .is_some_and(|graph| {
            graph
                .nodes
                .iter()
                .any(|node| matches!(node.kind, super::AnimKind::Size | super::AnimKind::Offset))
        })
        || control.children.iter().any(has_geometry)
}

/// Associates immutable control addresses with paths in the owned bound tree.
fn control_paths(
    control: &ResolvedControl,
    path: &mut Vec<usize>,
    out: &mut HashMap<usize, Vec<usize>>,
) {
    out.insert(std::ptr::from_ref(control).addr(), path.clone());
    for (index, child) in control.children.iter().enumerate() {
        path.push(index);
        control_paths(child, path, out);
        path.pop();
    }
}

/// Retains each visible control's own geometry program, including controls without primitives.
fn collect(laid: &LaidOut, paths: &HashMap<usize, Vec<usize>>, out: &mut Vec<Program>) {
    if !laid.visible {
        return;
    }
    if let Some(own) = laid.anim.as_ref().and_then(|anim| anim.own.as_ref())
        && own.moves()
    {
        out.push(Program {
            path: paths[&std::ptr::from_ref(laid.control).addr()].clone(),
            animation: Arc::clone(own),
        });
    }
    for child in &laid.children {
        collect(child, paths, out);
    }
}

/// Borrows one control by its stable child-index path.
fn at_ref<'a>(root: &'a ResolvedControl, path: &[usize]) -> &'a ResolvedControl {
    path.iter()
        .fold(root, |control, &index| &control.children[index])
}

/// Edits one control by its stable child-index path.
fn at<'a>(root: &'a mut ResolvedControl, path: &[usize]) -> &'a mut ResolvedControl {
    path.iter()
        .fold(root, |control, &index| &mut control.children[index])
}
