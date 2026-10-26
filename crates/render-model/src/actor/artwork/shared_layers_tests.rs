//! Packed session artwork references its compiled textures instead of keeping a second copy.
use super::*;

/// Distinct `side`×`side` textures for `sources`, so no two share pixels or a page-cache entry.
fn distinct_textures(sources: std::ops::Range<u32>, side: u16) -> Vec<assets::ActorTexture> {
    sources
        .map(|source| {
            let mut pixels: Vec<u8> = (0..usize::from(side) * usize::from(side) * 4)
                .map(|byte| byte as u8)
                .collect();
            pixels[..4].copy_from_slice(&source.to_le_bytes());
            let rgba8: Arc<[u8]> = pixels.into();
            assets::ActorTexture {
                source,
                width: side,
                height: side,
                pixel_sha256: Sha256::digest(&rgba8).into(),
                rgba8,
            }
        })
        .collect()
}

/// Bytes of every distinct allocation the textures and pages hold, counted once each.
fn retained_bytes(textures: &[assets::ActorTexture], pages: &ActorArtworkPages) -> usize {
    let mut seen = BTreeSet::new();
    textures
        .iter()
        .map(|texture| &texture.rgba8)
        .chain(pages.pages().iter().flat_map(|page| page.rgba8.parts()))
        .filter(|part| seen.insert(part.as_ptr()))
        .map(|part| part.len())
        .sum()
}

#[test]
fn packed_pack_artwork_retains_one_copy_of_each_texel() {
    // More same-size layers than one page holds, plus a second size, so grouping and splitting
    // both run.
    let mut textures = distinct_textures(0..300, 32);
    textures.extend(distinct_textures(300..305, 64));
    let texels: usize = textures
        .iter()
        .map(|texture| usize::from(texture.width) * usize::from(texture.height))
        .sum();
    let (pages, allocated) =
        test_alloc::measure(|| ActorArtworkPages::default().with_pack_artwork(&textures, &[]));
    assert!(pages.pages().len() >= 3);
    assert_eq!(
        pages.pages().iter().map(|page| page.layers()).sum::<u32>(),
        textures.len() as u32
    );
    assert_eq!(retained_bytes(&textures, &pages), texels * 4);
    // Page metadata stays well under one byte per texel; a pixel copy costs four.
    assert!(
        allocated.bytes < texels,
        "packing allocated {} bytes for {texels} texels",
        allocated.bytes
    );
    let rasters: Vec<_> = textures
        .iter()
        .map(|texture| EquipmentRaster {
            width: texture.width,
            height: texture.height,
            rgba8: Arc::clone(&texture.rgba8),
        })
        .collect();
    let (equipment, _) = ActorArtworkPages::default().with_equipment_rasters(&rasters);
    assert_eq!(retained_bytes(&textures, &equipment), texels * 4);
}
