//! Carrier pixels supplied to the shared texture preparation pipeline.

use super::{DecodedTexture, TextureImage};
use assets::{RuntimeAssets, material_keys::MaterialKeys};

/// A square donor's side, borrowed first-mip pixels, and optional baked tint.
type CarrierPixels<'a> = (u32, &'a [u8], Option<[u8; 3]>);

/// Reads only static donor pixels; baked animation timelines are not physical source strips.
pub(in super::super) fn carrier_image(
    base: &RuntimeAssets,
    keys: &MaterialKeys,
    path: &str,
    tint: Option<[u8; 3]>,
) -> Option<TextureImage> {
    let (size, pixels, baked_tint) = carrier_pixels(base, keys, path, tint)?;
    Some(TextureImage {
        image_path: path.to_ascii_lowercase(),
        texture: DecodedTexture {
            width: size,
            height: size,
            rgba8: pixels.into(),
        },
        baked_tint,
        pack: false,
    })
}

/// Resolves an available static donor's filename without copying its pixels.
pub(in super::super) fn carrier_image_path(
    base: &RuntimeAssets,
    keys: &MaterialKeys,
    path: &str,
    tint: Option<[u8; 3]>,
) -> Option<String> {
    carrier_pixels(base, keys, path, tint).map(|_| path.to_ascii_lowercase())
}

/// Selects the eligible donor and borrows its checked first-mip pixels with their baked tint.
fn carrier_pixels<'a>(
    base: &'a RuntimeAssets,
    keys: &MaterialKeys,
    path: &str,
    tint: Option<[u8; 3]>,
) -> Option<CarrierPixels<'a>> {
    let (id, source) = keys
        .material_sources()
        .filter(|(id, source)| {
            source.raster.eq_ignore_ascii_case(path)
                && base.material(*id).animation == assets::NO_ANIMATION
                && (source.baked_tint.is_none() || source.baked_tint == tint)
        })
        .min_by_key(|(_, source)| source.baked_tint != tint)?;
    let reference = base.material(id).texture;
    let mip = base
        .texture_pages()
        .get(reference.page() as usize)?
        .texture
        .mips
        .first()?;
    let bytes = (mip.size * mip.size * 4) as usize;
    let start = reference.layer() as usize * bytes;
    Some((
        mip.size,
        mip.rgba8.get(start..start + bytes)?,
        source.baked_tint,
    ))
}
