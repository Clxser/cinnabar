//! The Video renderer picker publishes only the renderers this build compiles.

use super::pack_label;
use crate::test_support::{centre, settings_view};
use launcher::menu::{MenuAction, MenuView, RendererChoice};
use render_api::EnhancedQuality;
use std::sync::Arc;
use ui::RenderMode;
use ui_engine::menu_scroll::MenuScrolls;

/// Every renderer and quality action the Video pane publishes while scrolling to its end.
fn renderer_actions(view: &MenuView) -> Vec<MenuAction> {
    let size = [1280.0, 720.0];
    let mut scrolls = MenuScrolls::<MenuAction>::default();
    let mut seen = Vec::new();
    for _ in 0..512 {
        let (areas, hits, _) = crate::review_tests::paint(scrolls.offsets().clone(), |canvas| {
            crate::settings::draw(canvas, view, size, &|_| None, None).unwrap();
        });
        for (action, _) in &hits {
            if matches!(
                action,
                MenuAction::SelectRenderer(_) | MenuAction::SetEnhancedQuality(_)
            ) && !seen.contains(action)
            {
                seen.push(*action);
            }
        }
        let viewport = areas
            .iter()
            .max_by(|left, right| left.viewport.min().x().total_cmp(&right.viewport.min().x()))
            .expect("settings content scroll view")
            .viewport;
        scrolls.set_areas(areas);
        let before = scrolls.offsets().clone();
        scrolls.wheel(centre(viewport), -1.0, false);
        if *scrolls.offsets() == before {
            break;
        }
    }
    seen
}

fn view_with_packs() -> MenuView {
    let mut view = settings_view("video_forced_index");
    view.shader_packs = Arc::from(["Dusk.zip".to_owned(), "Lumen".to_owned()]);
    view
}

#[test]
fn renderer_choices_follow_the_compiled_features() {
    let actions = renderer_actions(&view_with_packs());
    let mut expected = Vec::new();
    if render_model::ENHANCED_RENDERING_ENABLED || render_model::SHADER_PACKS_ENABLED {
        expected.push(RendererChoice::Vanilla);
    }
    if render_model::ENHANCED_RENDERING_ENABLED {
        expected.push(RendererChoice::Enhanced);
    }
    if render_model::SHADER_PACKS_ENABLED {
        expected.extend([RendererChoice::ShaderPack(0), RendererChoice::ShaderPack(1)]);
    }
    let renderers: Vec<_> = actions
        .iter()
        .filter_map(|action| match action {
            MenuAction::SelectRenderer(choice) => Some(*choice),
            _ => None,
        })
        .collect();
    assert_eq!(renderers.len(), expected.len(), "{renderers:?}");
    assert!(expected.iter().all(|choice| renderers.contains(choice)));
    assert!(
        !actions
            .iter()
            .any(|action| matches!(action, MenuAction::SetEnhancedQuality(_))),
        "quality stays locked while Vanilla is the renderer"
    );
}

#[test]
fn enhanced_quality_is_selectable_only_while_enhanced_is_the_renderer() {
    let mut view = view_with_packs();
    view.render_mode = RenderMode::Enhanced;
    view.renderer = Some(RendererChoice::Enhanced);
    let qualities = renderer_actions(&view)
        .into_iter()
        .filter(|action| matches!(action, MenuAction::SetEnhancedQuality(_)))
        .count();
    let expected = if render_model::ENHANCED_RENDERING_ENABLED {
        EnhancedQuality::ALL.len()
    } else {
        0
    };
    assert_eq!(qualities, expected);
}

#[test]
fn pack_labels_drop_only_a_zip_extension() {
    assert_eq!(pack_label("Dusk.zip"), "Dusk");
    assert_eq!(pack_label("Dusk.ZIP"), "Dusk");
    assert_eq!(pack_label("Lumen"), "Lumen");
    assert_eq!(pack_label(".zip"), ".zip");
    assert_eq!(pack_label("Lumen v2.zipped"), "Lumen v2.zipped");
}
