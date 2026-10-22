//! The community catalog preview uses native Settings controls and session-only saves.

use super::super::{
    Content,
    resources::{actions, caption},
};
use crate::{
    theme::{self, BODY, CAPTION, SECTION_HEADER, TEXT, TEXT_DIMMER},
    widgets::{self, Variant},
};
use launcher::{
    marketplace::{Action, Listing, State, catalog},
    menu::{MenuAction, auth::AuthState},
};
use ui_engine::engine::screen::PaintError;

#[cfg(test)]
mod tests;

const PROTOTYPE_NOTICE: &str =
    "Synthetic prototype catalog. Saves last for this session; no mods are downloaded.";
const SYNC_NOTICE: &str = "Microsoft-linked library sync requires the marketplace service.";

/// Wraps local preview navigation for the existing Mods input router.
fn command(action: Action) -> MenuAction {
    MenuAction::Mods(launcher::mods::Action::Marketplace(action))
}

/// Draws public catalog entries or one public revision, never an unpublished detail.
pub(super) fn draw(content: &mut Content<'_, '_>) -> Result<(), PaintError> {
    let snapshot = content.view.mods.clone();
    let state = &snapshot.marketplace;
    if let Some(id) = state.details {
        return match catalog().published(id) {
            Some(item) => details(content, state, item),
            None => unavailable(content),
        };
    }
    browse(content, state)
}

/// Keeps browsing, the local library and installed mods in the same native toolbar.
fn navigation(content: &mut Content<'_, '_>, state: &State) -> Result<(), PaintError> {
    let saved = state
        .saved
        .iter()
        .filter(|id| catalog().published(**id).is_some())
        .count();
    let library = format!("Demo library ({saved})");
    controls(
        content,
        &[
            ("Browse", Some(!state.library_only), Action::Browse),
            (&library, Some(state.library_only), Action::Library),
            ("Installed mods", None, Action::Close),
        ],
    )?;
    content.y += content.canvas.r(1.2);
    Ok(())
}

/// Lays raised choices and actions into rows sized from their actual labels.
fn controls(
    content: &mut Content<'_, '_>,
    items: &[(&str, Option<bool>, Action)],
) -> Result<(), PaintError> {
    if items.is_empty() {
        return Ok(());
    }
    let [left, right] = content.inset();
    let width = (right - left).max(1.0);
    let gap = content.canvas.r(0.8);
    let minimum = items.iter().try_fold(0.0_f32, |minimum, (label, _, _)| {
        content
            .canvas
            .measure(label, BODY)
            .map(|width| minimum.max(width + content.canvas.r(3.2)))
    })?;
    let columns = (((width + gap) / (minimum + gap)).floor().max(1.0) as usize).min(items.len());
    let cell = (width - gap * columns.saturating_sub(1) as f32) / columns as f32;
    for row in items.chunks(columns) {
        let height = row.iter().try_fold(0.0_f32, |height, (label, _, _)| {
            widgets::choice_height(content.canvas, label, cell).map(|next| height.max(next))
        })?;
        for (index, (label, selected, action)) in row.iter().enumerate() {
            let x = left + index as f32 * (cell + gap);
            let bounds = [x, content.y, x + cell, content.y + height];
            let action = command(*action);
            if let Some(selected) = selected {
                widgets::choice(
                    content.canvas,
                    content.view,
                    bounds,
                    label,
                    *selected,
                    action,
                )?;
                widgets::choice_focus(content.canvas, content.view, bounds, *selected, action)?;
            } else {
                widgets::button(
                    content.canvas,
                    content.view,
                    bounds,
                    Variant::Secondary,
                    label,
                    Some(action),
                )?;
            }
        }
        content.y += height + gap;
    }
    Ok(())
}

/// Derives filter labels and indices from published categories in the shared catalog.
fn filters(content: &mut Content<'_, '_>, state: &State) -> Result<(), PaintError> {
    let categories = catalog().categories();
    let selected = state.category.filter(|index| *index < categories.len());
    let mut choices = vec![("All", Some(selected.is_none()), Action::Category(None))];
    choices.extend(categories.iter().enumerate().map(|(index, label)| {
        (
            *label,
            Some(selected == Some(index)),
            Action::Category(Some(index)),
        )
    }));
    controls(content, &choices)
}

/// Shows a scan-friendly shelf with creator, purpose and permissions on every entry.
fn browse(content: &mut Content<'_, '_>, state: &State) -> Result<(), PaintError> {
    content.heading("Marketplace", "Explore free community mods.")?;
    navigation(content, state)?;
    caption(content, PROTOTYPE_NOTICE)?;
    filters(content, state)?;
    let items = state.listings();
    section(
        content,
        &format!(
            "{} ({})",
            if state.library_only {
                "Saved previews"
            } else {
                "Free community mods"
            },
            items.len()
        ),
    )?;
    if items.is_empty() {
        empty_library(content, state)?;
    } else {
        for item in items {
            listing_row(content, item, state.saved.contains(&item.id))?;
        }
    }
    content.y += content.canvas.r(1.6);
    account(content)?;
    content.y += content.canvas.r(2.4);
    Ok(())
}

/// Explains an empty local library or filter and offers the corresponding recovery.
fn empty_library(content: &mut Content<'_, '_>, state: &State) -> Result<(), PaintError> {
    let filtered = state
        .category
        .is_some_and(|index| index < catalog().categories().len());
    paragraph(
        content,
        match (state.library_only, filtered) {
            (true, true) => "No saved previews in this category.",
            (true, false) => "Your demo library is empty.",
            (false, true) => "No community mods in this category.",
            (false, false) => "No community mods are available yet.",
        },
        BODY,
        TEXT,
    )?;
    caption(
        content,
        if state.library_only {
            "Open a mod's details and choose Save preview to try the library. Nothing is installed."
        } else {
            "The prototype shows only published listings. Check another category or return to installed mods."
        },
    )?;
    actions(
        content,
        &[(
            if filtered {
                "Show all categories"
            } else if !state.library_only {
                "Back to installed mods"
            } else {
                "Browse community mods"
            },
            Variant::Primary,
            Some(command(if filtered {
                Action::Category(None)
            } else if !state.library_only {
                Action::Close
            } else {
                Action::Browse
            })),
        )],
    )
}

/// Draws one identity-scoped list action with fully wrapped, readable metadata.
fn listing_row(
    content: &mut Content<'_, '_>,
    item: &Listing,
    saved: bool,
) -> Result<(), PaintError> {
    let [left, right] = content.inset();
    let pad = content.canvas.r(1.2);
    let gap = content.canvas.r(0.4);
    let width = (right - left - pad * 2.0).max(1.0);
    let status = if saved { "Free · Saved" } else { "Free" };
    let status_width = content.canvas.measure(status, CAPTION)?;
    let title_width = (width - status_width - content.canvas.r(1.6)).max(1.0);
    let byline = format!("By {} · {}", item.creator, item.category);
    let permissions = if item.permissions.is_empty() {
        "Permissions: none declared".to_owned()
    } else {
        format!("Permissions: {}", item.permissions.join(", "))
    };
    let detail_width = content.canvas.measure("View details", CAPTION)?;
    let permissions_width = (width - detail_width - content.canvas.r(1.6)).max(1.0);
    let lines = [
        (item.title.as_str(), SECTION_HEADER, TEXT, title_width),
        (byline.as_str(), CAPTION, TEXT_DIMMER, width),
        (item.summary.as_str(), BODY, TEXT, width),
        (
            permissions.as_str(),
            CAPTION,
            TEXT_DIMMER,
            permissions_width,
        ),
    ];
    let heights: Vec<_> = lines
        .iter()
        .map(|(text, style, _, width)| {
            content
                .canvas
                .measure_height(text, *width, *style)
                .map(|height| height.max(content.canvas.r(style.line)))
        })
        .collect::<Result<_, _>>()?;
    let height = heights.iter().sum::<f32>() + gap * 2.0 + pad * 2.0;
    let bounds = [left, content.y, right, content.y + height];
    widgets::row(
        content.canvas,
        content.view,
        bounds,
        false,
        Some(command(Action::Details(item.id))),
    )?;
    let mut y = content.y + pad;
    for (index, ((text, style, color, width), height)) in lines.iter().zip(heights).enumerate() {
        if index == lines.len() - 1 {
            content.canvas.text_line(
                "View details",
                [right - pad - detail_width, y],
                detail_width + 1.0,
                CAPTION,
                TEXT,
            )?;
        }
        content
            .canvas
            .text(text, [left + pad, y], *width, *style, *color, false)?;
        y += height + if index == 1 || index == 2 { gap } else { 0.0 };
    }
    content.canvas.text_line(
        status,
        [right - pad - status_width, content.y + pad],
        status_width + 1.0,
        CAPTION,
        if saved {
            theme::SUCCESS_TINT
        } else {
            TEXT_DIMMER
        },
    )?;
    content.y = bounds[3] + content.canvas.r(0.8);
    Ok(())
}

/// Shows the complete description and declared access before a local preview save.
fn details(content: &mut Content<'_, '_>, state: &State, item: &Listing) -> Result<(), PaintError> {
    content.heading(
        &item.title,
        &format!("By {} · {}", item.creator, item.category),
    )?;
    actions(
        content,
        &[
            (
                if state.library_only {
                    "Back to demo library"
                } else {
                    "Back to catalog"
                },
                Variant::Secondary,
                Some(command(Action::Back)),
            ),
            (
                "Installed mods",
                Variant::Secondary,
                Some(command(Action::Close)),
            ),
        ],
    )?;
    content.y += content.canvas.r(1.2);
    caption(content, PROTOTYPE_NOTICE)?;
    paragraph(
        content,
        &format!(
            "Free · Version {} · License: {}",
            item.version, item.license
        ),
        BODY,
        TEXT,
    )?;
    paragraph(content, &item.description, BODY, TEXT)?;
    let saved = state.saved.contains(&item.id);
    actions(
        content,
        &[(
            if saved {
                "Remove saved preview"
            } else {
                "Save preview"
            },
            if saved {
                Variant::Secondary
            } else {
                Variant::Primary
            },
            Some(command(Action::Save(item.id))),
        )],
    )?;
    content.y += content.canvas.r(0.8);
    caption(
        content,
        if saved {
            "Saved in your demo library for this session. This mod is not installed."
        } else {
            "Save this listing to your demo library. This does not install or enable the mod."
        },
    )?;
    section(content, "Permissions")?;
    caption(
        content,
        "Declared by this demonstration listing. Saving a preview grants no access.",
    )?;
    if item.permissions.is_empty() {
        paragraph(content, "No permissions declared.", BODY, TEXT)?;
    } else {
        for permission in &item.permissions {
            content.label(permission, "")?;
        }
    }
    content.y += content.canvas.r(1.6);
    account(content)?;
    content.y += content.canvas.r(2.4);
    Ok(())
}

/// Keeps invalid or no-longer-published identities out of the detail view.
fn unavailable(content: &mut Content<'_, '_>) -> Result<(), PaintError> {
    content.heading(
        "Preview unavailable",
        "This listing is not in the public catalog.",
    )?;
    actions(
        content,
        &[(
            "Back to catalog",
            Variant::Secondary,
            Some(command(Action::Back)),
        )],
    )
}

/// Separates shelves and permission information using the existing heading face.
fn section(content: &mut Content<'_, '_>, title: &str) -> Result<(), PaintError> {
    content.y += content.canvas.r(1.2);
    paragraph(content, title, SECTION_HEADER, TEXT)
}

/// Advances by measured text height so longer copy and compact windows stay readable.
fn paragraph(
    content: &mut Content<'_, '_>,
    text: &str,
    style: theme::Type,
    color: theme::Rgba,
) -> Result<(), PaintError> {
    let [left, right] = content.inset();
    let height = content.canvas.text(
        text,
        [left, content.y],
        (right - left).max(1.0),
        style,
        color,
        false,
    )?;
    content.y += height.max(content.canvas.r(style.line)) + content.canvas.r(0.8);
    Ok(())
}

/// Reflects only the launcher's real account state without offering simulated sign-in.
fn account(content: &mut Content<'_, '_>) -> Result<(), PaintError> {
    let status = match &content.view.auth_state {
        AuthState::Authenticated => format!(
            "Microsoft account: {}. Demo saves last for this session.",
            launcher::menu::profile::current_name(content.view)
        ),
        AuthState::SignedOut => {
            "Browsing without a linked account. Demo saves last for this session.".to_owned()
        }
        AuthState::Checking => {
            "Checking Microsoft account. Demo saves last for this session.".to_owned()
        }
        AuthState::AwaitingCode { .. } | AuthState::AwaitingXboxSignup { .. } => {
            "Microsoft sign-in is in progress. Demo saves last for this session.".to_owned()
        }
        AuthState::Failed(_) => {
            "Microsoft sign-in is unavailable. Demo saves last for this session.".to_owned()
        }
    };
    caption(content, &status)?;
    caption(content, SYNC_NOTICE)
}
