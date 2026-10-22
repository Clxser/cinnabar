//! Immutable graph data and transient value framing stay outside the saved project.

use recording_studio::{
    Project,
    graph::{GraphChannel, GraphControl, GraphCurve, GraphKey},
};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphAction {
    Toggle,
    Channel(GraphChannel),
    Select(GraphKey),
    Edit(GraphField),
    Apply,
    Cancel,
    AddKey,
    DeleteKey,
    CycleEasing,
    Reset,
    Fit,
    ZoomValue(i8),
    PanValue(i8),
    Nudge {
        time: i8,
        value: i8,
    },
    Drag {
        key: GraphKey,
        control: GraphControl,
        bounds: [i32; 4],
    },
    Scrub {
        bounds: [i32; 4],
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphField {
    Time,
    Value,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GraphWorkspace {
    pub open: bool,
    pub channel: Option<GraphChannel>,
    pub selected: Option<GraphKey>,
    pub range: Option<[f64; 2]>,
    pub publication: Option<GraphPublication>,
    pub draft: Option<(GraphField, Arc<str>)>,
    pub plot: Option<GraphPlot>,
}

#[derive(Clone)]
pub struct GraphPlot {
    pub curve: Arc<GraphCurve>,
    pub window: [recording_studio::MediaTime; 2],
    pub points: Arc<[[f64; 2]]>,
}
impl PartialEq for GraphPlot {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.curve, &other.curve)
            && self.window == other.window
            && Arc::ptr_eq(&self.points, &other.points)
    }
}
impl std::fmt::Debug for GraphPlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GraphPlot")
            .field("window", &self.window)
            .field("points", &self.points.len())
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub struct GraphPublication {
    pub project: Arc<Project>,
    pub curve: Arc<GraphCurve>,
}

impl PartialEq for GraphPublication {
    /// Equality never traverses long curve arrays during ordinary UI frames.
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.project, &other.project) && Arc::ptr_eq(&self.curve, &other.curve)
    }
}

impl std::fmt::Debug for GraphPublication {
    /// Recorded curve contents remain out of ordinary debug logs.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GraphPublication")
            .field("item", &self.curve.item)
            .field("channel", &self.curve.channel)
            .finish_non_exhaustive()
    }
}
