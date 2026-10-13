//! Original fixed terrain data shared by Criterion and device tuning.

use assets::{
    BlockFlags, BlockOverlay, BlockVisual, ContributorRole, LightProperties, Material,
    NO_ANIMATION, NO_MODEL_TEMPLATE, RuntimeAssets, TextureRef, VisualKind, VisualSupport,
};
use world::{RawBlockIds, SubChunk};

pub const AIR: u32 = 0;
/// Encodes a fixture palette integer with Bedrock network zig-zag framing.
pub fn zig_zag_i32(value: i32) -> Vec<u8> {
    let mut value = ((value as u32) << 1) ^ ((value >> 31) as u32);
    let mut encoded = Vec::new();
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        encoded.push(byte);
        if value == 0 {
            return encoded;
        }
    }
}

// Synthetic opaque cubes use the diagnostic texture, not diagnostic block semantics.
// This deliberately measures no model or liquid workloads.
/// Creates original opaque cube materials for the fixed terrain fixture.
pub fn terrain_assets() -> RuntimeAssets {
    terrain_assets_with_hashes(Vec::new())
}

/// Terrain cube states at sequential ids 1 and 2; with `hashes`, one state per hash.
pub fn terrain_assets_with_hashes(hashes: Vec<Option<u32>>) -> RuntimeAssets {
    let base = RuntimeAssets::diagnostic();
    let mut texture = base.texture_array().clone();
    texture.layers = 1;
    let visual = |material| BlockVisual {
        native_material: assets::native_material::NativeMaterialFacts::Unresolved,
        faces: [material; 6],
        flags: BlockFlags::CUBE_GEOMETRY | BlockFlags::OCCLUDES_FULL_FACE,
        kind: VisualKind::Cube,
        support: VisualSupport::VanillaFallback,
        contributor_role: ContributorRole::Primary,
        model_template: NO_MODEL_TEMPLATE,
        animation: NO_ANIMATION,
        variant: 0,
        destroy: assets::compiled::block_destroy::BlockDestroyFacts::Unresolved,
    };
    let material = Material {
        texture: TextureRef::new(1, 0).unwrap(),
        flags: 0,
        ..Material::unvaried()
    };
    let states = hashes.len().max(2);
    let mut visuals = vec![visual(0), visual(1)];
    visuals.resize(states, visual(0));
    base.with_block_overlay(
        1,
        &BlockOverlay {
            visuals,
            light_properties: vec![LightProperties::OPAQUE_DARK; states],
            materials: vec![material; 2],
            textures: vec![texture],
            hashes,
            ..Default::default()
        },
    )
    .unwrap()
}

/// Decodes a section containing one network block state.
pub fn uniform_chunk(id: u32) -> SubChunk {
    let mut payload = vec![9, 1, 0, 1];
    payload.extend(zig_zag_i32(id as i32));
    SubChunk::decode(&payload, &RawBlockIds { air: AIR })
}

/// Returns the palette index for the fixed undulating terrain surface.
pub fn mixed_id(x: u8, y: u8, z: u8) -> u32 {
    let height = 4 + (u32::from(x) * 3 + u32::from(z) * 5) % 8;
    if u32::from(y) >= height {
        AIR
    } else if y < 3 {
        1
    } else {
        2
    }
}

/// Scattered blocks in open air, as in sections above the surface or in large caves.
pub fn sparse_id(x: u8, y: u8, z: u8) -> u32 {
    match (u32::from(x) * 7 + u32::from(y) * 3 + u32::from(z) * 5) % 23 {
        0 => 1,
        1 => 2,
        _ => AIR,
    }
}

/// Builds the fixed two-material terrain section.
pub fn mixed_chunk() -> SubChunk {
    chunk_of(mixed_id)
}

/// Builds a section from palette indices for air and two cube materials.
pub fn chunk_of(id_at: fn(u8, u8, u8) -> u32) -> SubChunk {
    chunk_with_palette(id_at, [AIR, 1, 2])
}

/// `id_at` picks a palette index; `palette` maps it to network values.
pub fn chunk_with_palette(id_at: fn(u8, u8, u8) -> u32, palette: [u32; 3]) -> SubChunk {
    // Bedrock v9, one network storage, two bits/index, x-z-y ordering.
    let mut payload = vec![9, 1, 0, 5];
    let mut words = [0_u32; 256];
    for x in 0..16_u8 {
        for z in 0..16_u8 {
            for y in 0..16_u8 {
                let linear = (usize::from(x) << 8) | (usize::from(z) << 4) | usize::from(y);
                words[linear / 16] |= id_at(x, y, z) << ((linear % 16) * 2);
            }
        }
    }
    for word in words {
        payload.extend_from_slice(&word.to_le_bytes());
    }
    payload.extend(zig_zag_i32(3));
    for id in palette {
        payload.extend(zig_zag_i32(id as i32));
    }
    let chunk = SubChunk::decode(&payload, &RawBlockIds { air: AIR });
    for x in 0..16_u8 {
        for z in 0..16_u8 {
            for y in 0..16_u8 {
                assert_eq!(
                    chunk.runtime_id(0, x, y, z),
                    Some(palette[id_at(x, y, z) as usize])
                );
            }
        }
    }
    chunk
}
