//! Measured chrome is reserved before the viewer and timeline share the remaining height.

use super::*;

pub(super) struct ViewerMetrics {
    pub pad: f32,
    pub transport: f32,
    pub toolbar: f32,
    pub notice: f32,
}

impl ViewerMetrics {
    /// Excludes padding and control rows from the space available for actual media pixels.
    pub(super) fn picture_height(&self, height: f32) -> f32 {
        (height - self.transport - self.toolbar - self.notice - self.pad * 2.0).max(0.0)
    }
    fn chrome(&self) -> f32 {
        self.transport + self.toolbar + self.notice + self.pad * 2.0
    }
}

/// The same measurements determine allocation and the painted picture/transport boundaries.
pub(super) fn viewer_metrics(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    width: f32,
) -> Result<ViewerMetrics, PaintError> {
    let pad = canvas.r(0.8);
    Ok(ViewerMetrics {
        pad,
        transport: transport::required_height(canvas, view, (width - pad * 2.0).max(0.0))? + pad,
        toolbar: viewer::required_height(canvas, view, width)?,
        notice: if view.studio.preview.notice.is_some() {
            canvas.r(2.5)
        } else {
            0.0
        },
    })
}

pub(super) struct Vertical {
    pub stage: Bounds,
    pub timeline: Bounds,
}

/// Requested sizes remain preferences; content and a usable picture bound them without changing modes.
pub(super) fn vertical(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
    viewer_width: f32,
) -> Result<Vertical, PaintError> {
    let available = (area[3] - area[1]).max(0.0);
    if view.studio.workspace.preview_only {
        return Ok(Vertical {
            stage: area,
            timeline: [area[0], area[3], area[2], area[3]],
        });
    }
    let gap = canvas.r(0.9).min(available);
    let viewer = viewer_metrics(canvas, view, viewer_width)?;
    let tracks = timeline::layout::metrics(canvas);
    let useful = tracks.useful_height(view.studio.lanes.tracks().len());
    let picture_floor = canvas.r(BODY.line * 7.5).max(120.0);
    let maximum = (available - gap - viewer.chrome() - picture_floor).min(useful);
    let preferred = available * view.studio.workspace.timeline_size.fraction();
    let height = if maximum >= tracks.minimum() {
        preferred.clamp(tracks.minimum(), maximum)
    } else if available >= gap + viewer.chrome() + tracks.minimum() {
        tracks.minimum()
    } else if available >= gap + viewer.chrome() + tracks.compact_minimum() {
        tracks.compact_minimum()
    } else {
        // Oversized GUI scales can make the floors physically impossible; keep controls inside the window.
        (available - gap - viewer.transport - viewer.pad)
            .max(0.0)
            .min(tracks.compact_minimum())
    };
    let top = (area[3] - height).max(area[1]);
    Ok(Vertical {
        stage: [area[0], area[1], area[2], (top - gap).max(area[1])],
        timeline: [area[0], top, area[2], area[3]],
    })
}

#[cfg(test)]
mod tests;
