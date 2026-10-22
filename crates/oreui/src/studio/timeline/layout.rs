//! The timeline's row, ruler and overview measurements are shared by paint and editor allocation.

use super::*;

pub(in crate::studio) struct Metrics {
    pub row: f32,
    pub ruler: f32,
    pub footer: f32,
    pub toolbar: f32,
    pub toolbar_pad: f32,
    pub toolbar_button: f32,
}

impl Metrics {
    /// One complete lane retains its ruler, editing tools and overview.
    pub(in crate::studio) fn minimum(&self) -> f32 {
        self.useful_height(1)
    }
    /// A tight timeline keeps a complete lane, ruler and overview without its optional editing row.
    pub(in crate::studio) fn compact_minimum(&self) -> f32 {
        self.toolbar_pad + self.ruler + self.footer + self.row
    }
    /// Empty authoring rows are real lanes; no space is reserved below the last published row.
    pub(in crate::studio) fn useful_height(&self, rows: usize) -> f32 {
        self.toolbar + self.ruler + self.footer + self.row * rows.max(1) as f32
    }
}

/// All painted geometry is measured at the active Studio density and native GUI scale.
pub(in crate::studio) fn metrics(canvas: &Canvas<'_>) -> Metrics {
    let toolbar_pad = canvas.r(0.6);
    let toolbar_button = canvas.r(3.8);
    Metrics {
        row: canvas.r(4.2),
        ruler: canvas.r(2.8),
        footer: canvas.r(3.6),
        toolbar: toolbar_button + toolbar_pad * 2.0,
        toolbar_pad,
        toolbar_button,
    }
}
