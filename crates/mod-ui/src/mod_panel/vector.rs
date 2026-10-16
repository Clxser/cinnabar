//! Generic two-axis pointer input backed by two existing bounded slider controls.
use super::*;

/// Captured axis routes retain their initial values for relative dragging.
#[derive(Clone)]
pub(super) struct Drag {
    action: String,
    origin: [f64; 2],
    values: Option<[f32; 2]>,
}

impl ModPanel {
    /// Captures authored vector rectangles and commits both stepped axes, including release.
    pub(super) fn vector_pointer(
        &mut self,
        point: [f64; 2],
        pressed: bool,
        held: bool,
    ) -> Option<Vec<Event>> {
        let frame = self.frame.as_ref()?;
        let selected = pressed
            .then(|| {
                frame.hits.iter().rev().find(|r| {
                    r.enabled
                        && r.contains(point)
                        && r.pressed.as_deref().is_some_and(|a| {
                            a.starts_with("mod.vector:")
                                || a.starts_with("mod.axis:")
                                || a.starts_with("mod.drag:")
                        })
                })
            })
            .flatten()
            .and_then(|r| r.pressed.clone());
        if pressed {
            self.vector_drag = selected.and_then(|action| {
                let values = if let Some(route) = action.strip_prefix("mod.drag:") {
                    let (x, y) = route.split_once(':')?;
                    let value =
                        |index: &str| match self.panel.controls.get(index.parse::<usize>().ok()?) {
                            Some(Control::Slider { value, .. }) => Some(*value),
                            _ => None,
                        };
                    Some([value(x)?, value(y)?])
                } else {
                    None
                };
                Some(Drag {
                    action,
                    origin: point,
                    values,
                })
            });
        }
        let drag = self.vector_drag.clone()?;
        let action = &drag.action;
        let Some(region) = frame
            .hits
            .iter()
            .find(|r| r.pressed.as_deref() == Some(action.as_str()))
        else {
            self.vector_drag = None;
            return Some(Vec::new());
        };
        let indices: [Option<usize>; 2] = if let Some(vector) = action
            .strip_prefix("mod.vector:")
            .or_else(|| action.strip_prefix("mod.drag:"))
        {
            let (x, y) = vector.split_once(':')?;
            [Some(x.parse().ok()?), Some(y.parse().ok()?)]
        } else if let Some(axis) = action.strip_prefix("mod.axis:") {
            let (index, axis) = axis.split_once(':')?;
            let index = Some(index.parse().ok()?);
            if axis == "y" {
                [None, index]
            } else {
                [index, None]
            }
        } else {
            self.vector_drag = None;
            return Some(Vec::new());
        };
        let local = region.local(point);
        let fractions = [
            region.fraction_at(local[0]),
            if region.rect.h > 0. {
                ((local[1] - region.rect.y) / region.rect.h).clamp(0., 1.)
            } else {
                0.
            },
        ];
        let mut events = Vec::new();
        let extent = self
            .panel
            .reference_size
            .map(|size| size.map(f64::from))
            .unwrap_or(self.viewport);
        for (axis, (index, fraction)) in indices.into_iter().zip(fractions).enumerate() {
            let Some(index) = index else {
                continue;
            };
            let Some(Control::Slider {
                id,
                value,
                min,
                max,
                step,
                ..
            }) = self.panel.controls.get_mut(index)
            else {
                continue;
            };
            let raw = if let Some(initial) = drag.values {
                initial[axis]
                    + ((point[axis] - drag.origin[axis]) / extent[axis].max(1.0)) as f32
                        * (*max - *min)
            } else {
                *min + fraction as f32 * (*max - *min)
            };
            let next = (*min + ((raw - *min) / *step).round() * *step).clamp(*min, *max);
            if *value != next {
                *value = next;
                events.push(Event {
                    id: id.clone(),
                    value: next,
                });
            }
        }
        if !held {
            self.vector_drag = None;
        }
        self.refresh_data();
        Some(events)
    }
}
