use super::{Page, raster};
use render_model::{UiRenderVertex, UiTexturePage};

/// Projects the source onto a square with the requested physical footprint.
fn vertices(side: f32) -> [UiRenderVertex; 4] {
    [[0., 0.], [1., 0.], [1., 1.], [0., 1.]].map(|point| UiRenderVertex {
        position: [20. + point[0] * side, 20. + point[1] * side],
        uv: point.map(|value| value * 16.),
        clip_z: 0.,
        clip_w: 1.,
        color: [255; 4],
        style_flags: 0,
        alpha_cutoff: -1.,
        model_light: 1.,
        overlay_color: [0.; 4],
    })
}

#[test]
fn minified_inventory_surfaces_average_source_texels_without_blurring_magnified_pixels() {
    let Some(gpu) = crate::gpu_snapshot::Gpu::for_fixture("inventory texture minification") else {
        return;
    };
    let pixels = (0..16)
        .flat_map(|y| {
            (0..16).flat_map(move |x| {
                let value = if (x + y) % 2 == 0 { 0 } else { 255 };
                [value, value, value, 255]
            })
        })
        .collect::<Vec<_>>();
    let source = UiTexturePage::owned([16; 2], pixels.into())
        .unwrap()
        .with_terrain_mips()
        .unwrap();
    let average = source.mip_levels().nth(2).unwrap().1[..4].to_vec();
    let reference = average.repeat(16 * 16);
    let indices = [0, 1, 2, 0, 2, 3];
    let draw = |side, page, flags| raster(&gpu, &vertices(side), None, &indices, page, flags);
    let source_page = || Page {
        pixels: source.pixels(),
        size: source.dimensions(),
        coverage: false,
        mips: Some(&source),
    };
    let expected = draw(
        4.,
        Page {
            pixels: &reference,
            size: [16; 2],
            coverage: false,
            mips: None,
        },
        0,
    );
    assert!(draw(4., source_page(), 0) != expected);
    assert!(
        draw(4., source_page(), render_api::UI_STYLE_TERRAIN_MIPS) == expected,
        "a four-to-one surface must sample its averaged source level",
    );
    assert!(
        draw(64., source_page(), render_api::UI_STYLE_TERRAIN_MIPS) == draw(64., source_page(), 0),
        "magnification keeps original nearest-filtered texels",
    );
}

#[test]
fn minified_cutouts_keep_partial_coverage() {
    let Some(gpu) = crate::gpu_snapshot::Gpu::for_fixture("inventory cutout minification") else {
        return;
    };
    let pixels = (0..16 * 16)
        .flat_map(|pixel| [255, 255, 255, if pixel % 4 == 0 { 255 } else { 0 }])
        .collect::<Vec<_>>();
    let source = UiTexturePage::owned([16; 2], pixels.into())
        .unwrap()
        .with_terrain_mips()
        .unwrap();
    let average = source.mip_levels().nth(2).unwrap().1[..4].repeat(16 * 16);
    let mut quad = vertices(4.);
    for vertex in &mut quad {
        vertex.alpha_cutoff = assets::gui_item::GUI_BLOCK_SAMPLE_ALPHA_THRESHOLD;
    }
    let indices = [0, 1, 2, 0, 2, 3];
    let reference = raster(
        &gpu,
        &quad,
        None,
        &indices,
        Page {
            pixels: &average,
            size: [16; 2],
            coverage: false,
            mips: None,
        },
        0,
    );
    let actual = raster(
        &gpu,
        &quad,
        None,
        &indices,
        Page {
            pixels: source.pixels(),
            size: source.dimensions(),
            coverage: false,
            mips: Some(&source),
        },
        render_api::UI_STYLE_TERRAIN_MIPS,
    );
    assert!(actual == reference);
    for vertex in &mut quad {
        vertex.alpha_cutoff = f32::from(assets::gui_item::GUI_BLOCK_ALPHA_THRESHOLD) / 255.;
    }
    assert!(
        raster(
            &gpu,
            &quad,
            None,
            &indices,
            Page {
                pixels: source.pixels(),
                size: source.dimensions(),
                coverage: false,
                mips: Some(&source)
            },
            render_api::UI_STYLE_TERRAIN_MIPS,
        ) != reference,
        "a source-texel cutoff erases minified coverage"
    );
}
