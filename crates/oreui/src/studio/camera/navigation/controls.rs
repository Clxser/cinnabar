//! Small inspection controls never imply a saved shot edit.

use super::*;
use launcher::menu::studio::CameraNavigationAction as N;

/// Speed and response remain usable while a view refresh renders; leveling requires a native image.
pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
) -> Result<f32, PaintError> {
    let [left, y, right, _] = area;
    let gap = canvas.r(0.4);
    let height = canvas.r(3.8);
    let settings = view.studio.camera_workspace.navigation;
    let decimals = if settings.speed() < 9.995 {
        2
    } else if settings.speed() < 99.95 {
        1
    } else {
        0
    };
    let speed = format!("{:.*} b/s", decimals, settings.speed());
    let field = canvas.measure(&speed, crate::theme::SECONDARY_BUTTON)? + canvas.r(1.2);
    let small = canvas
        .r(3.0)
        .min(((right - left - field - gap * 2.0) * 0.5).max(canvas.r(1.6)));
    for (bounds, label, action) in [
        ([left, y, left + small, y + height], "-", N::Speed(-1)),
        (
            [left + small + gap, y, right - small - gap, y + height],
            speed.as_str(),
            N::ResetSpeed,
        ),
        ([right - small, y, right, y + height], "+", N::Speed(1)),
    ] {
        control(
            canvas,
            view,
            bounds,
            label,
            Action::CameraNavigation(action),
            true,
            Variant::Secondary,
        )?;
    }
    let y = y + height + gap;
    let middle = (left + right) * 0.5;
    control(
        canvas,
        view,
        [left, y, middle - gap * 0.5, y + height],
        "Smooth",
        Action::CameraNavigation(N::ToggleSmooth),
        true,
        if settings.smooth {
            Variant::Primary
        } else {
            Variant::Secondary
        },
    )?;
    control(
        canvas,
        view,
        [middle + gap * 0.5, y, right, y + height],
        "Level",
        Action::CameraNavigation(N::Level),
        view.studio
            .preview
            .camera
            .as_ref()
            .is_some_and(|preview| preview.interactive),
        Variant::Secondary,
    )?;
    Ok(y + canvas.r(4.8))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn navigation_controls_remain_full_targets_in_a_compact_native_panel() {
        let view = MenuView::new(true, "Fixture".into());
        for width in [150.0, 220.0] {
            let mut full = Vec::new();
            let (_, hits, _) =
                crate::review_tests::paint(std::collections::HashMap::new(), |canvas| {
                    canvas.capture_focus = true;
                    draw(canvas, &view, [0.0, 0.0, width, 200.0]).unwrap();
                    full = canvas.focus_hits.clone();
                });
            for action in [N::Speed(-1), N::Speed(1), N::ResetSpeed, N::ToggleSmooth] {
                let action = MenuAction::Studio(Action::CameraNavigation(action));
                let bounds = hits.iter().find(|(found, _)| *found == action).unwrap().1;
                assert!(bounds.min().x() >= 0.0 && bounds.max().x() <= width);
                assert_eq!(
                    bounds,
                    full.iter().find(|(found, _)| *found == action).unwrap().1
                );
            }
            assert!(!hits.iter().any(
                |(action, _)| *action == MenuAction::Studio(Action::CameraNavigation(N::Level))
            ));
        }
    }
}
