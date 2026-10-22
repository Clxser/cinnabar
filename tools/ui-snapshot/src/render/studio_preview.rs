//! Original perspective-room artwork isolates camera controls without a game capture or recording.

use jsonui_editor::raster::Page;
use launcher::menu::{MenuScreen, MenuView, studio::ViewportTool};

const SIZE: [u32; 2] = [960, 540];

/// Rotation fixtures need a real image so the production fitted-image overlay is exercised.
pub(super) fn install(view: &mut MenuView, pages: &mut Vec<Page>) {
    if view.screen != MenuScreen::RecordingStudio
        || view.studio.workspace.viewport_tool != ViewportTool::Rotate
        || view.studio.preview_icon.is_some()
    {
        return;
    }
    let mut pixels = Vec::with_capacity((SIZE[0] * SIZE[1] * 4) as usize);
    for y in 0..SIZE[1] {
        for x in 0..SIZE[0] {
            // A geometric courtyard is generated here; no installed or captured game assets enter it.
            let u = (f64::from(x) + 0.5) / f64::from(SIZE[0]) - 0.5;
            let v = (f64::from(y) + 0.5) / f64::from(SIZE[1]);
            let color = if v < 0.43 {
                [117, 152, 165, 255]
            } else {
                let depth = 0.22 / (v - 0.42);
                let side = u * depth * 1.8;
                let grid = (side.floor() as i64 + depth.floor() as i64).rem_euclid(2);
                if side.abs() > 2.4 {
                    if grid == 0 {
                        [96, 113, 91, 255]
                    } else {
                        [109, 128, 103, 255]
                    }
                } else if grid == 0 {
                    [165, 160, 140, 255]
                } else {
                    [146, 145, 128, 255]
                }
            };
            pixels.extend_from_slice(&color);
        }
    }
    let icon = ui::IconRef {
        page: pages.len() as u16,
        uv: [0, 0, SIZE[0] as u16, SIZE[1] as u16],
        glint: false,
    };
    pages.push(Page {
        repeat_content: None,
        width: SIZE[0],
        height: SIZE[1],
        rgba: pixels.into(),
    });
    std::sync::Arc::make_mut(&mut view.studio).preview_icon = Some(icon);
}
