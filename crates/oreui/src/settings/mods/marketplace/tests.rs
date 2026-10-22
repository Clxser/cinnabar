use super::*;
use crate::paint::Canvas;
use launcher::{menu::MenuView, mods};
use std::sync::Arc;

struct Painted {
    hits: Vec<(MenuAction, ui::UiRect)>,
    focus: Vec<MenuAction>,
    text: String,
}

/// Normalizes wrapped glyph text without depending on how text nodes are split.
fn words(text: &str) -> String {
    text.chars()
        .filter(|character| !character.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Projects local marketplace state into the same Mods screen used by the client.
fn view(state: State) -> MenuView {
    let mut view = MenuView::new(true, "Local player".into());
    Arc::make_mut(&mut view.mods).marketplace = state;
    view
}

/// Paints native hit and focus targets without runtime artwork or account services.
fn render(view: &MenuView, width: f32) -> Painted {
    let (mut nodes, mut next, mut layouts) =
        (Vec::new(), 1, ui::TextLayoutCache::new(128, 1024 * 1024));
    let font = ui_engine::test_support::fixture_font();
    let metrics = ui_engine::text_metrics::TextMetrics::for_viewport(
        [1280, 720],
        ui::DpiScale::new(1.0).unwrap(),
        Some(2),
    );
    let (hits, focus) = {
        let mut canvas = Canvas::new(&mut nodes, &mut next, &mut layouts, &font, metrics, 0, None);
        canvas.capture_focus = true;
        let mut content = Content {
            canvas: &mut canvas,
            view,
            span: [100.0, 100.0 + width],
            column_width: width,
            y: 100.0,
            translate: &|_| None,
            nested: false,
        };
        super::super::draw(&mut content).unwrap();
        (
            std::mem::take(&mut canvas.hits),
            canvas
                .focus_targets
                .iter()
                .map(|target| target.action)
                .collect(),
        )
    };
    let text = nodes
        .iter()
        .filter_map(|node| match node.visual() {
            ui::UiVisual::Text { layout, .. } => Some(
                layout
                    .glyphs()
                    .iter()
                    .map(|glyph| glyph.codepoint)
                    .collect::<String>(),
            ),
            _ => None,
        })
        .collect::<String>();
    Painted {
        hits,
        focus,
        text: words(&text),
    }
}

/// Returns a browsing state through the public local-navigation reducer.
fn browsing() -> State {
    let mut state = State::default();
    state.apply(Action::Browse);
    state
}

/// Extracts listing identities offered by the native catalog rows.
fn detail_ids(painted: &Painted) -> Vec<u64> {
    painted
        .hits
        .iter()
        .filter_map(|(action, _)| match action {
            MenuAction::Mods(mods::Action::Marketplace(Action::Details(id))) => Some(*id),
            _ => None,
        })
        .collect()
}

#[test]
fn installed_mods_offer_the_marketplace_and_opening_it_replaces_install_controls() {
    let installed = render(&view(State::default()), 800.0);
    assert!(
        installed
            .hits
            .iter()
            .any(|(action, _)| *action == command(Action::Browse))
    );
    let browser = render(&view(browsing()), 800.0);
    assert!(
        browser
            .hits
            .iter()
            .any(|(action, _)| *action == command(Action::Close))
    );
    assert!(
        browser
            .hits
            .iter()
            .all(|(action, _)| matches!(action, MenuAction::Mods(mods::Action::Marketplace(_))))
    );
    assert!(browser.text.contains(&words(PROTOTYPE_NOTICE)));
    assert!(!browser.text.contains(&words("Import mod")));
}

#[test]
fn unpublished_and_unknown_identities_never_become_public_rows_or_details() {
    let catalog = catalog();
    let hidden: Vec<_> = catalog
        .listings
        .iter()
        .filter(|item| catalog.published(item.id).is_none())
        .collect();
    assert!(
        hidden.iter().any(|item| item.status == "pending"),
        "fixture needs a pending submission"
    );
    let unknown = catalog.listings.iter().map(|item| item.id).max().unwrap() + 1;
    let mut state = browsing();
    state.saved.extend(hidden.iter().map(|item| item.id));
    state.saved.insert(unknown);
    let browser = render(&view(state.clone()), 800.0);
    for item in &hidden {
        assert!(!browser.text.contains(&words(&item.title)));
        assert!(!detail_ids(&browser).contains(&item.id));
    }
    assert!(!detail_ids(&browser).contains(&unknown));
    for id in hidden.iter().map(|item| item.id).chain([unknown]) {
        state.details = Some(id);
        let unavailable = render(&view(state.clone()), 800.0);
        assert!(unavailable.text.contains(&words("Preview unavailable")));
        assert!(
            unavailable
                .hits
                .iter()
                .all(|(action, _)| *action == command(Action::Back))
        );
        for item in &hidden {
            assert!(!unavailable.text.contains(&words(&item.title)));
        }
    }
    state.apply(Action::Library);
    let library = render(&view(state), 800.0);
    assert!(detail_ids(&library).is_empty());
    assert!(library.text.contains(&words("Demo library (0)")));
}

#[test]
fn category_filters_keep_listing_identity_in_every_selection_action() {
    let categories = catalog().categories();
    for selected in std::iter::once(None).chain((0..categories.len()).map(Some)) {
        let mut state = browsing();
        state.apply(Action::Category(selected));
        let expected: Vec<_> = state.listings().iter().map(|item| item.id).collect();
        let painted = render(&view(state), 800.0);
        assert_eq!(detail_ids(&painted), expected);
        for index in 0..categories.len() {
            assert!(
                painted
                    .hits
                    .iter()
                    .any(|(action, _)| *action == command(Action::Category(Some(index))))
            );
        }
        assert!(
            painted
                .hits
                .iter()
                .any(|(action, _)| *action == command(Action::Category(None)))
        );
    }
}

#[test]
fn details_show_the_complete_description_permissions_and_only_a_preview_save() {
    for item in browsing().listings() {
        let mut state = browsing();
        state.apply(Action::Details(item.id));
        for saved in [false, true] {
            if saved {
                state.apply(Action::Save(item.id));
            }
            let painted = render(&view(state.clone()), 800.0);
            for text in [&item.description, &item.version, &item.license] {
                assert!(painted.text.contains(&words(text)));
            }
            for permission in &item.permissions {
                assert!(painted.text.contains(&words(permission)));
            }
            assert!(
                painted
                    .text
                    .contains(&words("Saving a preview grants no access."))
            );
            assert!(painted.text.contains(&words(SYNC_NOTICE)));
            assert!(painted.text.contains(&words(if saved {
                "Remove saved preview"
            } else {
                "Save preview"
            })));
            assert!(
                painted
                    .hits
                    .iter()
                    .any(|(action, _)| *action == command(Action::Save(item.id)))
            );
            assert!(painted.hits.iter().all(|(action, _)| matches!(
                action,
                MenuAction::Mods(mods::Action::Marketplace(_))
            )));
        }
    }
}

#[test]
fn saved_preview_can_be_opened_from_the_library_and_removed_back_to_empty() {
    let mut state = browsing();
    let first = detail_ids(&render(&view(state.clone()), 800.0))[0];
    state.apply(Action::Details(first));
    state.apply(Action::Save(first));
    state.apply(Action::Library);
    assert_eq!(detail_ids(&render(&view(state.clone()), 800.0)), [first]);
    state.apply(Action::Details(first));
    let detail = render(&view(state.clone()), 800.0);
    assert!(detail.text.contains(&words("Back to demo library")));
    assert!(detail.text.contains(&words("Remove saved preview")));
    state.apply(Action::Save(first));
    state.apply(Action::Back);
    let empty = render(&view(state), 800.0);
    assert!(detail_ids(&empty).is_empty());
    assert!(empty.text.contains(&words("Your demo library is empty.")));
    assert!(empty.text.contains(&words("Nothing is installed.")));
    assert!(
        empty
            .hits
            .iter()
            .any(|(action, _)| *action == command(Action::Browse))
    );
}

#[test]
fn native_controls_remain_focusable_and_inside_the_content_on_compact_windows() {
    let mut state = browsing();
    let id = state.listings()[0].id;
    for width in [300.0, 800.0] {
        for action in [Action::Browse, Action::Library, Action::Details(id)] {
            state.apply(action);
            let mut view = view(state.clone());
            view.focused_action = Some(command(action));
            view.navigation_focus_visible = true;
            let painted = render(&view, width);
            assert!(!painted.hits.is_empty());
            for (action, bounds) in &painted.hits {
                assert!(
                    painted.focus.contains(action),
                    "missing controller target for {action:?}"
                );
                assert!(bounds.min().x() >= 100.0);
                assert!(bounds.max().x() <= 100.0 + width);
                assert!(bounds.width() > 0.0 && bounds.height() > 0.0);
            }
        }
    }
}

#[test]
fn account_copy_uses_real_auth_state_and_never_claims_service_sync() {
    let mut view = view(browsing());
    view.feeds.profile.gamertag = "Example signed-in player".into();
    let anonymous = render(&view, 800.0);
    assert!(
        anonymous
            .text
            .contains(&words("Browsing without a linked account."))
    );
    assert!(
        !anonymous
            .text
            .contains(&words(&view.feeds.profile.gamertag))
    );
    view.auth_state = AuthState::Authenticated;
    let authenticated = render(&view, 800.0);
    assert!(
        authenticated
            .text
            .contains(&words("Microsoft account: Example signed-in player."))
    );
    assert!(authenticated.text.contains(&words(SYNC_NOTICE)));
    for auth in [
        AuthState::SignedOut,
        AuthState::Checking,
        AuthState::Failed("Synthetic diagnostic".into()),
    ] {
        view.auth_state = auth;
        let painted = render(&view, 800.0);
        assert!(!painted.text.contains(&words(&view.feeds.profile.gamertag)));
        assert!(!painted.text.contains(&words("Synthetic diagnostic")));
        assert!(
            !painted
                .hits
                .iter()
                .any(|(action, _)| *action == MenuAction::StartSignIn)
        );
    }
}
