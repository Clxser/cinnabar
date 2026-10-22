//! Bounded normalized line meshes shared by camera and scalar-graph overlays.

use super::*;
use std::sync::Arc;
use ui::{UiBlendMode, UiMesh, UiMeshBatch, UiMeshVertex};

#[derive(Default)]
pub(super) struct Lines {
    vertices: Vec<UiMeshVertex>,
    indices: Vec<u32>,
}

impl Lines {
    /// Builds one anti-gap quad per projected line, clamped to the image's normalized bounds.
    pub(super) fn push(
        &mut self,
        bounds: Bounds,
        segment: [[f64; 2]; 2],
        color: [u8; 4],
        thickness: f32,
    ) {
        let Some(segment) = clip(segment) else {
            return;
        };
        let [a, b] = segment.map(|point| {
            screen(
                [0.0, 0.0, bounds[2] - bounds[0], bounds[3] - bounds[1]],
                point,
            )
        });
        let delta = [b[0] - a[0], b[1] - a[1]];
        let length = delta[0].hypot(delta[1]);
        if length < 0.1 {
            return;
        }
        let normal = [
            -delta[1] / length * thickness * 0.5,
            delta[0] / length * thickness * 0.5,
        ];
        let base = self.vertices.len() as u32;
        for (point, sign) in [(a, -1.0), (a, 1.0), (b, 1.0), (b, -1.0)] {
            self.vertices.push(UiMeshVertex {
                position: [
                    ((point[0] + normal[0] * sign) / (bounds[2] - bounds[0])).clamp(0.0, 1.0),
                    ((point[1] + normal[1] * sign) / (bounds[3] - bounds[1])).clamp(0.0, 1.0),
                ],
                clip_z: 0.0,
                clip_w: 1.0,
                uv: [0.0; 2],
                color,
                model_light: 1.0,
                overlay_color: [0.0; 4],
                style_flags: 0,
                alpha_test: false,
            });
        }
        self.indices
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    /// One native UI mesh draws the entire bounded rail behind its interactive controls.
    pub(super) fn draw(
        mut self,
        canvas: &mut Canvas<'_>,
        bounds: Bounds,
    ) -> Result<(), PaintError> {
        if self.indices.is_empty() {
            return Ok(());
        }
        for vertex in &mut self.vertices {
            vertex.color[3] = (f32::from(vertex.color[3]) * canvas.alpha).round() as u8;
        }
        let count = self.indices.len() as u32;
        let mesh = UiMesh::new(
            self.vertices.into(),
            self.indices.into(),
            Arc::from([UiMeshBatch {
                texture_page: canvas.solid_page,
                index_range: 0..count,
                blend: UiBlendMode::Alpha,
                depth_test: false,
                depth_write: false,
                alpha_cutoff: None,
            }]),
        )
        .map_err(|_| PaintError::Tree(ui::UiError::DrawIndexOverflow))?;
        canvas.mesh(bounds, Arc::new(mesh))
    }
}

/// Liang–Barsky clipping rejects off-range segments before thick mesh quads are constructed.
fn clip(segment: [[f64; 2]; 2]) -> Option<[[f64; 2]; 2]> {
    if segment.iter().flatten().any(|v| !v.is_finite()) {
        return None;
    }
    let [a, b] = segment;
    let delta = [b[0] - a[0], b[1] - a[1]];
    if !delta.iter().all(|v| v.is_finite()) {
        return None;
    }
    let (mut low, mut high) = (0.0_f64, 1.0_f64);
    for axis in 0..2 {
        if delta[axis] == 0.0 {
            if !(0.0..=1.0).contains(&a[axis]) {
                return None;
            }
        } else {
            let t0 = -a[axis] / delta[axis];
            let t1 = (1.0 - a[axis]) / delta[axis];
            low = low.max(t0.min(t1));
            high = high.min(t0.max(t1));
            if low > high {
                return None;
            }
        }
    }
    Some(
        [low, high]
            .map(|t| std::array::from_fn(|axis| (a[axis] + delta[axis] * t).clamp(0.0, 1.0))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn outside_segments_do_not_create_false_plot_border_curves() {
        assert!(clip([[0.0, -2.0], [1.0, -1.0]]).is_none());
        assert!(clip([[0.0, 2.0], [1.0, 3.0]]).is_none());
        assert_eq!(
            clip([[-0.5, -1.0], [1.5, 1.0]]),
            Some([[0.5, 0.0], [1.0, 0.5]])
        );
    }
}

/// Maps normalized endpoints into local pixel space before forming a thick quad.
fn screen(bounds: Bounds, point: [f64; 2]) -> [f32; 2] {
    [
        bounds[0] + (bounds[2] - bounds[0]) * point[0] as f32,
        bounds[1] + (bounds[3] - bounds[1]) * point[1] as f32,
    ]
}
