//! Recovery when the normal native studio paint fails; the open project remains retained.

use super::*;

pub(super) fn fallback(
    view: &MenuView,
    nodes: &mut Vec<UiNode>,
    hits: &mut Vec<(MenuAction, UiRect)>,
    next: &mut u32,
    layouts: &mut TextLayoutCache,
    font: &assets::RuntimeFontCatalog,
    metrics: TextMetrics,
    solid_page: u16,
    area: ContentArea,
) -> Result<(), UiPresentationError> {
    text(
        nodes,
        next,
        layouts,
        font,
        metrics,
        solid_page,
        "Recording Studio could not be displayed. Your open project is still retained.",
        [area.left + SPACE_LG, area.top + SPACE_LG],
        area.width - SPACE_LG * 2.0,
        TEXT,
    )?;
    button(
        view,
        nodes,
        hits,
        next,
        layouts,
        font,
        metrics,
        solid_page,
        MenuAction::AddBack,
        usize::MAX,
        "Back",
        area.left + SPACE_LG,
        area.top + 100.0,
        180.0,
        TOUCH_CONTROL_HEIGHT,
        SafeArea::ZERO,
    )
}
