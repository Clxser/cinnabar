//! The uploaded block-entity lists the shader pack draws: alpha-tested models, blended
//! layers and the beacon beam, with the block-entity atlas.

use super::*;
use crate::shaderpack::actors::{BlockEntityList, BlockEntitySource};

/// The block-entity lists, once the atlas is uploaded.
pub(crate) fn shaderpack_block_entities(world: &World) -> Option<BlockEntitySource<'_>> {
    let gpu = world.get_resource::<BlockEntityGpu>()?;
    fn list(list: &VertexList) -> Option<BlockEntityList<'_>> {
        list.buffer
            .as_ref()
            .filter(|_| list.count != 0)
            .map(|buffer| BlockEntityList {
                buffer,
                count: list.count,
            })
    }
    Some(BlockEntitySource {
        atlas: gpu.texture.as_ref()?,
        solid: list(&gpu.solid),
        overlay: list(&gpu.overlay),
        additive: list(&gpu.additive),
    })
}
