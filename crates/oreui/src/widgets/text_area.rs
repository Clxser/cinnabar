//! A bounded visible slice of an explicit multiline field, with byte-accurate pointer placement.

use super::*;
use ui_engine::caret::caret_byte;

/// Follows the caret's explicit line and horizontally scrolls its ink inside one native field face.
#[allow(clippy::too_many_arguments)]
pub(in super::super) fn text_area(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    b: Bounds,
    value: &str,
    placeholder: &str,
    focused: bool,
    action: MenuAction,
) -> Result<(), PaintError> {
    let mut state = canvas.interaction(view, Some(action));
    state.focused |= focused;
    let motion = canvas.feedback(state, true, false, Kind::Field);
    let edge = canvas.r(EDGE);
    canvas.fill(b, BORDER)?;
    canvas.fill(
        [b[0] + edge, b[1] + edge, b[2] - edge, b[3] - edge],
        mix(
            canvas.appearance.surface(NEUTRAL80.fill),
            canvas.role(NEUTRAL80).hovered,
            motion.hover,
        ),
    )?;
    canvas.hit(action, b)?;
    let pad = canvas.r(1.0);
    let inner = [b[0] + pad, b[1] + pad, b[2] - pad, b[3] - pad];
    let clip = canvas.begin_clip(inner)?;
    let line_height = canvas.r(BODY.line);
    let rows = ((inner[3] - inner[1]) / line_height).floor().max(1.0) as usize;
    let byte = caret_byte(value, view.caret.byte);
    let caret_line = value[..byte].bytes().filter(|&v| v == b'\n').count();
    let first = if focused {
        caret_line.saturating_sub(rows - 1)
    } else {
        0
    };
    let mut offset = 0;
    for (index, line) in value.split('\n').enumerate() {
        let start = offset;
        let end = start + line.len();
        offset = end + 1;
        if index < first {
            continue;
        }
        if index >= first + rows {
            break;
        }
        let y = inner[1] + (index - first) as f32 * line_height;
        let row = [inner[0], y, inner[2], y + line_height];
        let local_caret = byte.saturating_sub(start).min(line.len());
        let caret_x = if focused && index == caret_line {
            canvas.measure(&line[..local_caret], BODY)?
        } else {
            0.0
        };
        let scroll = (caret_x - (inner[2] - inner[0] - edge * 2.0)).max(0.0);
        let x = inner[0] - scroll;
        if focused && let Some([a, b]) = view.caret.selection {
            let a = caret_byte(line, a.saturating_sub(start).min(line.len()));
            let b = caret_byte(line, b.saturating_sub(start).min(line.len()));
            if b > a {
                let left = x + canvas.measure(&line[..a], BODY)?;
                let right = x + canvas.measure(&line[..b], BODY)?;
                canvas.fill([left, y, right, y + line_height], NEUTRAL20.fill)?;
            }
        }
        canvas.text_line(
            if value.is_empty() { placeholder } else { line },
            [x, y],
            inner[2] - x,
            BODY,
            if value.is_empty() {
                FIELD_PLACEHOLDER
            } else {
                TEXT
            },
        )?;
        if focused && index == caret_line && view.caret.shown && view.caret.selection.is_none() {
            canvas.fill(
                [x + caret_x, y, x + caret_x + edge, y + line_height],
                FIELD_CARET,
            )?;
        }
        let count = canvas.hits.len();
        canvas.hit(action, row)?;
        if let Some(field) = action.text_field()
            && canvas.hits.len() > count
        {
            let bounds = canvas.hits.last().expect("admitted text row hit").1;
            let metrics = canvas.metrics;
            let request = canvas.text_request(line, 65536 * 64, BODY)?;
            let factor = request.scale.get() / metrics.scale.get();
            let letter_spacing_64 = request.wrap.letter_spacing_64;
            canvas.spots.push(TextSpot {
                layout: None,
                field,
                byte_range: Some([start, end]),
                bounds,
                left: x,
                factor,
                effect_scale_1024: 0,
                font: Some(BODY.face.name().into()),
                letter_spacing_64,
                metrics,
            });
        }
    }
    canvas.end_clip(clip);
    if motion.focus > 0.0 {
        canvas.frame(b, EDGE, opacity(OUTLINE, motion.focus))?;
    }
    Ok(())
}
