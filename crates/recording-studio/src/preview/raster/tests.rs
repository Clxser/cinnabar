use super::*;
use crate::model::FramingFit;

/// Synthetic RGBA texels need neither a codec process nor a game asset fixture.
fn frame(size: [u32; 2], pixels: &[[u8; 4]]) -> DecodedFrame {
    DecodedFrame {
        size,
        rgba8: pixels.iter().flatten().copied().collect::<Vec<_>>().into(),
    }
}

/// Tests inspect linear composition as well as final sRGB bytes without reading back a GPU.
fn composed(source: &DecodedFrame, size: [u32; 2], framing: Framing) -> Raster {
    let mut target = Raster::new(size).unwrap();
    target.overlay(source, framing, false, &|| false).unwrap();
    target
}

#[test]
fn clockwise_quarter_turn_places_source_texels_in_the_expected_rows() {
    let red = [255, 0, 0, 255];
    let green = [0, 255, 0, 255];
    let blue = [0, 0, 255, 255];
    let yellow = [255, 255, 0, 255];
    let cyan = [0, 255, 255, 255];
    let magenta = [255, 0, 255, 255];
    let white = [255; 4];
    let black = [0, 0, 0, 255];
    let source = frame(
        [4, 2],
        &[red, green, blue, yellow, cyan, magenta, white, black],
    );
    let target = composed(
        &source,
        [2, 4],
        Framing {
            rotation_degrees: 90.0,
            ..Framing::default()
        },
    );
    let encoded = target.finish(&|| false).unwrap();
    let expected: Vec<_> = [cyan, red, magenta, green, white, blue, black, yellow]
        .into_iter()
        .flatten()
        .collect();
    for (actual, expected) in encoded.iter().zip(expected) {
        assert!(actual.abs_diff(expected) <= 1);
    }
}

#[test]
fn fill_removes_letterbox_and_rotated_corner_bars_while_fit_preserves_the_whole_crop() {
    let source = frame([4, 2], &[[255; 4]; 8]);
    let fit = composed(&source, [16, 16], Framing::default());
    assert_eq!(fit.linear[0], [0.0, 0.0, 0.0, 1.0]);
    for rotation_degrees in [0.0, 31.25, -45.0, 90.0] {
        let fill = composed(
            &source,
            [16, 16],
            Framing {
                rotation_degrees,
                fit: FramingFit::Fill,
                ..Framing::default()
            },
        );
        assert!(
            fill.linear
                .iter()
                .all(|pixel| pixel.iter().all(|value| (*value - 1.0).abs() < 0.00001))
        );
    }
}

#[test]
fn rotated_cropped_filtering_keeps_opacity_and_ignores_transparent_pixel_colour() {
    let source = frame([2, 1], &[[255, 0, 0, 0], [0, 0, 255, 255]]);
    let blended = sample(&source, [0.5, 0.5], srgb_table());
    assert_eq!(blended, [0.0, 0.0, 0.5, 0.5]);
    let framing = Framing {
        crop: [0.5, 0.0, 1.0, 1.0],
        rotation_degrees: 33.0,
        fit: FramingFit::Fill,
        opacity: 0.5,
    };
    let target = composed(&source, [8, 8], framing);
    assert!(target.linear.iter().all(|pixel| pixel[0] == 0.0
        && pixel[1] == 0.0
        && (pixel[2] - 0.5).abs() < 0.00001
        && pixel[3] == 1.0));
}
