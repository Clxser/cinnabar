//! A full-size scalar graph replaces the track area while the replay viewer remains available.

use super::lines::Lines;
use super::*;
use launcher::menu::studio::graph::{GraphAction, GraphField};
use recording_studio::graph::{GraphControl, GraphKey};

const CURVE: [u8; 4] = [160, 224, 129, 255];
const GRID: [u8; 4] = [61, 70, 77, 255];

#[cfg(test)]
mod tests;

/// Native controls, graph coordinates and pointer actions share the same content rectangle.
pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
) -> Result<(), PaintError> {
    if area[2] <= area[0] || area[3] <= area[1] {
        return Ok(());
    }
    widgets::panel(canvas, area)?;
    let workspace = &view.studio.camera_workspace.graph;
    let pad = canvas.r(0.7);
    let height = canvas.r(3.4);
    let mut x = area[0] + pad;
    let y = area[1] + pad;
    let button = canvas.r(7.2);
    control(
        canvas,
        view,
        [x, y, x + button, y + height],
        "Tracks",
        Action::Graph(GraphAction::Toggle),
        true,
        Variant::Secondary,
    )?;
    x += button + pad;
    let channels = view
        .studio
        .project
        .as_ref()
        .zip(view.studio.selected)
        .map_or_else(Vec::new, |(project, item)| {
            recording_studio::graph::channels(project, item)
        });
    for channel in channels {
        let width = (canvas.measure(channel.label(), CAPTION)? + canvas.r(2.0)).max(canvas.r(4.2));
        if x + width > area[2] - pad {
            break;
        }
        control(
            canvas,
            view,
            [x, y, x + width, y + height],
            channel.label(),
            Action::Graph(GraphAction::Channel(channel)),
            true,
            if workspace.channel == Some(channel) {
                Variant::Primary
            } else {
                Variant::Neutral
            },
        )?;
        x += width + pad;
    }
    let Some(publication) = workspace.publication.as_ref() else {
        return canvas.text_centred(
            "Select a camera, effect, or clip with speed keys.",
            [area[0], y + height, area[2], area[3]],
            CAPTION,
            TEXT_DIMMER,
            false,
        );
    };
    let curve = &publication.curve;
    let selection = workspace
        .selected
        .filter(|key| key.item == curve.item && key.channel == curve.channel);
    let locked = view
        .studio
        .project
        .as_ref()
        .and_then(|project| recording_studio::tracks::item_track(project, curve.item).ok())
        .is_some_and(|track| view.studio.workspace.timeline.locked.contains(&track));
    let y = y + height + pad;
    let mut x = area[0] + pad;
    for (label, action, enabled) in [
        ("+ Key", GraphAction::AddKey, !locked),
        (
            "Delete",
            GraphAction::DeleteKey,
            selection.is_some() && !locked,
        ),
        (
            "Ease",
            GraphAction::CycleEasing,
            selection.is_some() && !locked,
        ),
        ("Fit", GraphAction::Fit, true),
        ("Reset", GraphAction::Reset, !locked),
        ("Y +", GraphAction::ZoomValue(1), true),
        ("Y -", GraphAction::ZoomValue(-1), true),
        ("Up", GraphAction::PanValue(1), true),
        ("Down", GraphAction::PanValue(-1), true),
    ] {
        let width = canvas.measure(label, CAPTION)? + canvas.r(2.2);
        if x + width > area[2] - pad {
            break;
        }
        control(
            canvas,
            view,
            [x, y, x + width, y + height],
            label,
            Action::Graph(action),
            enabled,
            Variant::Neutral,
        )?;
        x += width + pad;
    }
    let mut plot_top = y + height + canvas.r(1.2);
    if let Some(point) = selection.and_then(|key| curve.points.iter().find(|p| p.at == key.at)) {
        let mut x = area[0] + pad;
        let field_width = canvas.r(13.0).min((area[2] - x - pad) * 0.28);
        for (field, value, placeholder) in [
            (GraphField::Time, point.at.seconds(), "Time s"),
            (
                GraphField::Value,
                if curve.channel == recording_studio::graph::GraphChannel::Speed {
                    point.value.exp2()
                } else {
                    point.value
                },
                curve.channel.label(),
            ),
        ] {
            let label_width = canvas.measure(placeholder, CAPTION)? + canvas.r(0.8);
            canvas.text_line(
                placeholder,
                [x, plot_top + canvas.r(0.7)],
                label_width,
                CAPTION,
                TEXT_DIMMER,
            )?;
            x += label_width;
            let text = scalar_label(value);
            let draft = workspace
                .draft
                .as_ref()
                .filter(|(f, _)| *f == field)
                .map(|(_, s)| s.as_ref());
            widgets::text_field(
                canvas,
                view,
                [x, plot_top, x + field_width, plot_top + height],
                draft.unwrap_or(&text),
                placeholder,
                view.field == Some(launcher::menu::MenuField::StudioGraph(field)),
                (!locked).then_some(MenuAction::Studio(Action::Graph(GraphAction::Edit(field)))),
            )?;
            x += field_width + pad;
        }
        let label = if workspace.draft.is_some() {
            "Apply"
        } else {
            point.easing.label()
        };
        let width = canvas.measure(label, CAPTION)? + canvas.r(2.2);
        if x + width <= area[2] - pad {
            control(
                canvas,
                view,
                [x, plot_top, x + width, plot_top + height],
                label,
                Action::Graph(if workspace.draft.is_some() {
                    GraphAction::Apply
                } else {
                    GraphAction::CycleEasing
                }),
                !locked,
                Variant::Secondary,
            )?;
            x += width + pad;
        }
        if workspace.draft.is_some() {
            let width = canvas.measure("Cancel", CAPTION)? + canvas.r(2.2);
            if x + width <= area[2] - pad {
                control(
                    canvas,
                    view,
                    [x, plot_top, x + width, plot_top + height],
                    "Cancel",
                    Action::Graph(GraphAction::Cancel),
                    true,
                    Variant::Neutral,
                )?;
            }
        }
        plot_top += height + pad;
    }
    let plot = [
        area[0] + canvas.r(8.8),
        plot_top,
        area[2] - canvas.r(1.5),
        area[3] - canvas.r(3.0),
    ];
    if plot[2] - plot[0] < 30.0 || plot[3] - plot[1] < 20.0 {
        return Ok(());
    }
    let bounds = plot.map(|v| v.round() as i32);
    canvas.hit(
        MenuAction::Studio(Action::Graph(GraphAction::Scrub { bounds })),
        plot,
    )?;
    let range = workspace.range.unwrap_or(curve.range);
    let start = view.studio.visible_start.0 as f64;
    let duration = view.studio.visible_duration.0.max(1) as f64;
    let normalized = |at: f64, value: f64| {
        [
            (at - start) / duration,
            1.0 - recording_studio::graph::range_fraction(range, value),
        ]
    };
    let mut lines = Lines::default();
    for tick in 0..=4 {
        let fraction = f64::from(tick) / 4.0;
        lines.push(plot, [[0.0, fraction], [1.0, fraction]], GRID, 1.0);
        lines.push(plot, [[fraction, 0.0], [fraction, 1.0]], GRID, 1.0);
        let value = recording_studio::graph::range_value(range, 1.0 - fraction);
        let label = if curve.channel == recording_studio::graph::GraphChannel::Speed {
            format!("{:.2}x", value.exp2())
        } else {
            scalar_label(value)
        };
        canvas.text_line(
            &label,
            [
                area[0] + pad,
                plot[1] + (plot[3] - plot[1]) * fraction as f32 - canvas.r(0.8),
            ],
            canvas.r(7.7),
            CAPTION,
            TEXT_DIMMER,
        )?;
        canvas.text_line(
            &format!("{:.2}s", (start + duration * fraction) / 1_000_000.0),
            [
                (plot[0] + (plot[2] - plot[0]) * fraction as f32 - canvas.r(2.0))
                    .clamp(plot[0], (plot[2] - canvas.r(8.0)).max(plot[0])),
                plot[3] + canvas.r(0.5),
            ],
            canvas.r(8.0),
            CAPTION,
            TEXT_DIMMER,
        )?;
    }
    if let Some(samples) = workspace
        .plot
        .as_ref()
        .filter(|samples| std::sync::Arc::ptr_eq(&samples.curve, curve))
    {
        for pair in samples.points.windows(2) {
            let previous = normalized(pair[0][0], pair[0][1]);
            let next = normalized(pair[1][0], pair[1][1]);
            if previous.iter().chain(next.iter()).all(|v| v.is_finite()) {
                lines.push(plot, [previous, next], CURVE, 1.8);
            }
        }
    }
    let playhead = (view.studio.position.0 as f64 - start) / duration;
    if (0.0..=1.0).contains(&playhead) {
        lines.push(plot, [[playhead, 0.0], [playhead, 1.0]], TEXT, 1.0);
    }
    let mut handle_markers = Vec::with_capacity(2);
    if let Some(key) = selection
        && let Some(handles) = curve.handles(key.at)
    {
        let index = curve.points.partition_point(|p| p.at < key.at);
        let a = curve.points[index - 1];
        let b = curve.points[index];
        for (handle, owner, kind) in [
            (handles[0], a, GraphControl::Outgoing),
            (handles[1], b, GraphControl::Incoming),
        ] {
            let p = normalized(handle[0], handle[1]);
            lines.push(
                plot,
                [normalized(owner.at.0 as f64, owner.value), p],
                TEXT_DIMMER,
                1.0,
            );
            handle_markers.push((
                p,
                (!locked).then_some(GraphAction::Drag {
                    key,
                    control: kind,
                    bounds,
                }),
            ));
        }
    }
    lines.draw(canvas, plot)?;
    for (point, action) in handle_markers {
        marker(canvas, plot, point, [220, 189, 113, 255], action)?;
    }
    if curve.relative_values() {
        canvas.text_line(
            "Offsets from recorded subject rig",
            [plot[0] + canvas.r(0.7), plot[1] + canvas.r(0.7)],
            plot[2] - plot[0] - canvas.r(1.4),
            CAPTION,
            TEXT_DIMMER,
        )?;
    }
    let mut previous = -20.0;
    let selected_at = selection.map(|key| key.at);
    for point in &curve.points {
        let p = normalized(point.at.0 as f64, point.value);
        let x = p[0] * (plot[2] - plot[0]) as f64;
        let selected = selected_at == Some(point.at);
        if !selected && x - previous < 10.0 {
            continue;
        }
        previous = x;
        let key = GraphKey {
            item: curve.item,
            channel: curve.channel,
            at: point.at,
        };
        marker(
            canvas,
            plot,
            p,
            if selected { TEXT } else { CURVE },
            (!locked).then_some(GraphAction::Drag {
                key,
                control: GraphControl::Key,
                bounds,
            }),
        )?;
    }
    Ok(())
}

/// Axis and key readouts keep very small or large finite scene values legible within native fields.
fn scalar_label(value: f64) -> String {
    if value.abs() >= 1e6 || (value != 0.0 && value.abs() < 1e-3) {
        format!("{value:.3e}")
    } else {
        format!("{value:.5}")
    }
}

/// A handle never installs an input target outside the visible graph or through its neighboring toolbar.
fn marker(
    canvas: &mut Canvas<'_>,
    plot: Bounds,
    p: [f64; 2],
    color: [u8; 4],
    action: Option<GraphAction>,
) -> Result<(), PaintError> {
    if p.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
        return Ok(());
    }
    let x = plot[0] + (plot[2] - plot[0]) * p[0] as f32;
    let y = plot[1] + (plot[3] - plot[1]) * p[1] as f32;
    let half = canvas.r(0.6).max(4.0);
    let bounds = [
        (x - half).max(plot[0]),
        (y - half).max(plot[1]),
        (x + half).min(plot[2]),
        (y + half).min(plot[3]),
    ];
    canvas.fill(bounds, color)?;
    if let Some(action) = action {
        canvas.hit(MenuAction::Studio(Action::Graph(action)), bounds)?;
    }
    Ok(())
}
