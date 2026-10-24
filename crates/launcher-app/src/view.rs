//! Asset-independent OreUI drawing of the launcher window with the shared menu theme.

use bootstrap_ui::{
    canvas::{Canvas, Image, Rect, Text},
    view::{ButtonState, backdrop, button_label, centred, panel_surface, raised_button},
};
use ui::oreui_theme::{self as theme, Appearance};

use crate::screen::{Action, Overview, Screen};

/// Body lines beyond this are cut, so a long error cannot push the buttons off the window.
const MAX_BODY_LINES: usize = 10;

/// How one frame draws: scale, theme and the interaction state of its buttons.
pub struct Style<'a> {
    /// Physical pixels per rem, from the same GUI-scale authority as the menus.
    pub rem: f32,
    pub appearance: Appearance,
    pub hovered: Option<Action>,
    pub focused: Option<Action>,
    pub pressed: Option<Action>,
    pub logo: Option<&'a Image>,
}

struct Layout {
    rem: f32,
    panel: Rect,
    logo: Rect,
    title: Vec<String>,
    body: Vec<String>,
    rows: usize,
    title_line: f32,
    body_line: f32,
}

/// Picks the largest scale up to the GUI scale at which the logo and panel fit the window.
fn layout(canvas: &Canvas, text: &mut Text, screen: &Screen, style: &Style<'_>) -> Layout {
    let (width, height) = (canvas.width as f32, canvas.height as f32);
    let mut rem = style.rem.max(1.0);
    loop {
        let margin = theme::SPACE[4] * rem;
        let pad = theme::LOADING_PAD * rem;
        let panel_w = ((theme::LOADING_WIDTH + theme::CANCEL_WIDTH) * rem)
            .min(width - margin * 2.0)
            .max(pad * 2.0 + rem);
        let inner = panel_w - pad * 2.0;
        let title_px = theme::HEADER5.size * rem;
        let body_px = theme::CAPTION.size * rem;
        let title = text.wrap(screen.title(), title_px, inner);
        let detail = screen.body();
        let mut body = if detail.is_empty() {
            Vec::new()
        } else {
            text.wrap(&detail, body_px, inner)
        };
        if body.len() > MAX_BODY_LINES {
            body.truncate(MAX_BODY_LINES);
            body.push("…".into());
        }
        let rows = if *screen == Screen::Home {
            Overview::ROWS
        } else {
            0
        };
        let title_line = (theme::HEADER5.line * rem).max(text.line_height(title_px));
        let body_line = (theme::BODY.line * rem).max(text.line_height(body_px));
        let lines = body.len() + rows;
        let panel_h = pad * 2.0
            + title.len() as f32 * title_line
            + if lines == 0 {
                0.0
            } else {
                theme::SPACE[3] * rem + lines as f32 * body_line
            }
            + if screen.actions().is_empty() {
                0.0
            } else {
                theme::LOADING_FOOTER_AREA * rem
            };
        let logo_w = if style.logo.is_some() {
            (theme::LOADING_WIDTH * 0.75 * rem).min(panel_w * 0.82)
        } else {
            0.0
        };
        let logo_h = style.logo.map_or(0.0, |logo| {
            logo_w * logo.height as f32 / logo.width.max(1) as f32
        });
        let logo_space = if style.logo.is_some() {
            logo_h + theme::SPACE[5] * rem
        } else {
            0.0
        };
        let group_h = panel_h + logo_space;
        if group_h + margin * 2.0 <= height || rem <= 1.0 {
            let top = ((height - group_h) * 0.5).max(0.0);
            return Layout {
                rem,
                panel: Rect {
                    x: (width - panel_w) * 0.5,
                    y: top + logo_space,
                    w: panel_w,
                    h: panel_h,
                },
                logo: Rect {
                    x: (width - logo_w) * 0.5,
                    y: top,
                    w: logo_w,
                    h: logo_h,
                },
                title,
                body,
                rows,
                title_line,
                body_line,
            };
        }
        rem = (rem * 0.9).max(1.0);
    }
}

/// Draws one launcher frame and returns the hit target of every button it shows.
pub fn draw(
    canvas: &mut Canvas,
    text: &mut Text,
    screen: &Screen,
    overview: &Overview,
    style: &Style<'_>,
) -> Vec<(Action, Rect)> {
    let layout = layout(canvas, text, screen, style);
    let Layout {
        rem,
        panel,
        title_line,
        body_line,
        ..
    } = layout;
    let appearance = style.appearance;
    backdrop(
        canvas,
        appearance,
        style.logo.map(|logo| (logo, layout.logo)),
    );
    panel_surface(canvas, appearance, panel, rem);
    let inner = panel.inset(theme::LOADING_PAD * rem);
    let mut y = inner.y;
    for line in &layout.title {
        centred(
            canvas,
            text,
            inner,
            y,
            theme::HEADER5.size * rem,
            theme::TEXT,
            line,
        );
        y += title_line;
    }
    if layout.rows + layout.body.len() > 0 {
        y += theme::SPACE[3] * rem;
    }
    let px = theme::CAPTION.size * rem;
    for (label, value) in overview.rows().iter().take(layout.rows) {
        y = row(canvas, text, inner, y, px, body_line, label, value);
    }
    for line in &layout.body {
        centred(canvas, text, inner, y, px, theme::TEXT_DIMMER, line);
        y += body_line;
    }
    let actions = screen.actions();
    if actions.is_empty() {
        return Vec::new();
    }
    y += theme::SPACE[4] * rem;
    buttons(canvas, text, screen, style, panel, inner, y, rem)
}

/// Draws a dimmed label on the left and its value on the right, cutting a long value in the
/// middle so both ends of a path stay readable; returns the next row's top.
#[expect(
    clippy::too_many_arguments,
    reason = "one row of the shared layout pass"
)]
fn row(
    canvas: &mut Canvas,
    text: &mut Text,
    inner: Rect,
    y: f32,
    px: f32,
    line: f32,
    label: &str,
    value: &str,
) -> f32 {
    let baseline = y + (line - text.line_height(px)) * 0.5;
    text.draw(canvas, inner.x, baseline, px, theme::TEXT_DIMMEST, label);
    // Keep at least one em between the label and its value.
    let room = inner.w - text.width(label, px) - px;
    let value = text.truncate_middle(value, px, room.max(0.0));
    let x = inner.x + inner.w - text.width(&value, px);
    text.draw(canvas, x, baseline, px, theme::TEXT, &value);
    y + line
}

/// Lays the screen's buttons out in one centred row, shrinking them together if they overflow.
#[expect(
    clippy::too_many_arguments,
    reason = "the footer of the shared layout pass"
)]
fn buttons(
    canvas: &mut Canvas,
    text: &mut Text,
    screen: &Screen,
    style: &Style<'_>,
    panel: Rect,
    inner: Rect,
    y: f32,
    rem: f32,
) -> Vec<(Action, Rect)> {
    let actions = screen.actions();
    let gap = theme::SPACE[1] * rem;
    let widths: Vec<_> = actions
        .iter()
        .map(|action| {
            (text.width(
                screen.button_label(*action),
                theme::SECONDARY_BUTTON.size * rem,
            ) + theme::SPACE[6] * rem)
                .max(theme::CANCEL_WIDTH * rem)
        })
        .collect();
    let total = widths.iter().sum::<f32>() + gap * (actions.len() - 1) as f32;
    let fit = (inner.w / total).min(1.0);
    let mut x = panel.x + (panel.w - total * fit) * 0.5;
    let mut hits = Vec::new();
    for (&action, width) in actions.iter().zip(widths) {
        let rect = Rect {
            x,
            y,
            w: width * fit,
            h: theme::BUTTON_HEIGHT * rem,
        };
        let role = style.appearance.role(if screen.primary() == Some(action) {
            theme::PRIMARY_ROLE
        } else {
            theme::MENU_NEUTRAL
        });
        let state = ButtonState {
            hovered: style.hovered == Some(action),
            focused: style.focused == Some(action),
            pressed: style.pressed == Some(action),
        };
        let face = raised_button(canvas, style.appearance, role, state, rect, rem);
        button_label(
            canvas,
            text,
            face,
            rem,
            role.text,
            screen.button_label(action),
        );
        hits.push((action, rect));
        x += (width + gap) * fit;
    }
    hits
}

#[cfg(test)]
mod tests {
    use update_manifest::channel::Channel;

    use super::*;

    /// Loads the open font directly, before any installed carriers exist.
    fn font() -> Text {
        Text::from_bytes(include_bytes!("../../../assets/fonts/CinnanglesSans.ttf")).unwrap()
    }

    #[test]
    fn every_screen_fits_small_and_large_windows() {
        let mut text = font();
        let logo = bootstrap_ui::decode_logo();
        let overview = Overview {
            version: "0.1.20".into(),
            channel: Channel::Beta,
            resources_ready: false,
            data_folder: "/home/someone/.local/share/cinnabar/with/a/rather/long/path".into(),
            development: false,
        };
        let screens = [
            Screen::Home,
            Screen::Launching,
            Screen::Failed {
                message: "word ".repeat(400),
            },
        ];
        for size in [[560, 420], [1024, 640], [2048, 1280]] {
            for offset in [-3, 0] {
                for screen in &screens {
                    let mut canvas = Canvas::new(size[0], size[1]);
                    let style = Style {
                        rem: f32::from(
                            ui::DesktopGuiScale::for_window(size).scale_for_offset(offset),
                        ) * theme::GUI_PIXELS_PER_REM,
                        appearance: Appearance::Default,
                        hovered: None,
                        focused: None,
                        pressed: None,
                        logo: logo.as_ref(),
                    };
                    let layout = layout(&canvas, &mut text, screen, &style);
                    assert!(layout.panel.y >= 0.0);
                    assert!(layout.panel.y + layout.panel.h <= size[1] as f32);
                    let hits = draw(&mut canvas, &mut text, screen, &overview, &style);
                    assert_eq!(
                        hits.iter().map(|(action, _)| *action).collect::<Vec<_>>(),
                        screen.actions()
                    );
                    for (_, rect) in hits {
                        assert!(rect.x >= layout.panel.x);
                        assert!(rect.x + rect.w <= layout.panel.x + layout.panel.w + 0.5);
                        assert!(rect.y + rect.h <= layout.panel.y + layout.panel.h + 0.5);
                    }
                }
            }
        }
    }
}
