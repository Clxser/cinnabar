//! Queued visual samples share immutable assets while retaining a bounded amount of changing data.

use std::{
    any::TypeId,
    collections::BTreeMap,
    sync::{
        Arc, Weak,
        atomic::{AtomicUsize, Ordering},
    },
};

use super::FrozenVisuals;

pub(super) const MAX_QUEUED_BYTES: usize = 256 * 1024 * 1024;
pub(super) const MAX_FRAME_BYTES: usize = 192 * 1024 * 1024;
const SAMPLE_OVERHEAD: usize = 512;

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
struct Allocation {
    kind: TypeId,
    address: usize,
    bytes: usize,
}

struct Charge {
    used: Arc<AtomicUsize>,
    bytes: usize,
    // Weak ownership keeps the allocation address reserved until every accounting lease retires.
    _identity: Option<Box<dyn Send + Sync>>,
}

impl Drop for Charge {
    fn drop(&mut self) {
        self.used.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

/// The encoder closure retains this lease until it has finished using its frozen sample.
#[derive(Default)]
pub(super) struct Lease {
    charges: Vec<Arc<Charge>>,
}

#[derive(Default)]
pub(super) struct Budget {
    used: Arc<AtomicUsize>,
    allocations: BTreeMap<Allocation, Weak<Charge>>,
}

impl Budget {
    /// Charges shared immutable catalogs once and the remaining draw data for every queued sample.
    pub(super) fn retain(&mut self, frame: &FrozenVisuals) -> Result<(Lease, usize), String> {
        let total = frame
            .retained_bytes()
            .checked_add(SAMPLE_OVERHEAD)
            .filter(|&bytes| bytes <= MAX_FRAME_BYTES)
            .ok_or("The prepared visual frame exceeds its memory budget.")?;
        self.allocations
            .retain(|_, charge| charge.strong_count() > 0);
        let mut lease = Lease::default();
        let mut shared = 0usize;

        if let Some(base) = &frame.particles.base {
            shared += self.shared(base, base.len(), &mut lease)?;
        }
        shared += self.allocation(&frame.particles.patches, &mut lease)?;
        for patch in frame.particles.patches.iter() {
            shared += self.shared(&patch.rgba8, patch.rgba8.len(), &mut lease)?;
        }
        for draws in [
            &frame.particles.opaque,
            &frame.particles.blend,
            &frame.particles.add,
        ] {
            shared += self.allocation(draws, &mut lease)?;
        }
        if let Some(received) = &frame.particles.received {
            shared += self.allocation(received, &mut lease)?;
        }
        if let Some(atlas) = &frame.blocks.atlas {
            shared += self.shared(&atlas.static_rgba8, atlas.static_rgba8.len(), &mut lease)?;
        }
        shared += self.allocation(&frame.blocks.dynamic_rgba8, &mut lease)?;
        for draws in [
            &frame.blocks.solid,
            &frame.blocks.overlay,
            &frame.blocks.crack,
            &frame.blocks.portal,
            &frame.blocks.additive,
        ] {
            shared += self.allocation(draws, &mut lease)?;
        }
        if let Some(scene) = &frame.nametags {
            let bytes = std::mem::size_of::<render_model::NametagScene>()
                + std::mem::size_of_val(&*scene.records)
                + std::mem::size_of_val(&*scene.labels);
            shared += self.shared(scene, bytes, &mut lease)?;
            shared += self.shared(
                &scene.atlas,
                std::mem::size_of_val(&*scene.atlas),
                &mut lease,
            )?;
            for cell in scene.atlas.iter() {
                shared += self.shared(&cell.rgba8, cell.rgba8.len(), &mut lease)?;
            }
        }
        if let Some(items) = &frame.items {
            shared += self.allocation(&items.models, &mut lease)?;
            for model in items.models.iter() {
                use render_model::DroppedItemModel;
                match model {
                    DroppedItemModel::Vacant => {}
                    DroppedItemModel::Sprite(sprite) | DroppedItemModel::NativeSprite(sprite) => {
                        shared += self.allocation(&sprite.rgba8, &mut lease)?;
                    }
                    DroppedItemModel::Cube(cube) => {
                        for face in &cube.faces {
                            shared += self.allocation(face, &mut lease)?;
                        }
                    }
                    DroppedItemModel::Block(block) => {
                        shared += self.allocation(&block.materials, &mut lease)?;
                        shared += self.allocation(&block.quads, &mut lease)?;
                        for (sprite, _) in block.materials.iter() {
                            shared += self.allocation(&sprite.rgba8, &mut lease)?;
                        }
                    }
                }
            }
            shared += self.allocation(&items.instances, &mut lease)?;
            shared += self.allocation(&items.terrain_instances, &mut lease)?;
            for instance in items.terrain_instances.iter() {
                shared += self.allocation(&instance.transitions, &mut lease)?;
            }
            shared += self.allocation(&items.dynamic, &mut lease)?;
            shared += self.allocation(&items.ropes, &mut lease)?;
            if let Some(terrain) = &items.terrain {
                shared += self.allocation(&terrain.generations, &mut lease)?;
            }
            if let Some(glint) = &items.glint {
                shared += self.shared(&glint.rgba8, glint.rgba8.len(), &mut lease)?;
            }
        }
        if let Some(hand) = &frame.hand {
            let vertices = &hand.rig.geometry_vertices;
            shared += self.shared(
                &vertices.segments,
                std::mem::size_of_val(&*vertices.segments),
                &mut lease,
            )?;
            for page in vertices.segments.iter() {
                shared += self.shared(page, std::mem::size_of_val(&**page), &mut lease)?;
            }
            shared += self.shared(
                &vertices.offsets,
                std::mem::size_of_val(&*vertices.offsets),
                &mut lease,
            )?;
            shared += self.shared(
                &hand.rig.geometry_spans,
                std::mem::size_of_val(&*hand.rig.geometry_spans),
                &mut lease,
            )?;
            shared += self.allocation(&hand.rig.instances, &mut lease)?;
            shared += self.allocation(&hand.rig.manifest, &mut lease)?;
            shared += self.allocation(&hand.rig.previous_bones, &mut lease)?;
            shared += self.allocation(&hand.rig.current_bones, &mut lease)?;
            shared += self.shared(hand.skin.pixels(), hand.skin.len(), &mut lease)?;
            for atlas in hand.item_atlases.iter().flatten() {
                shared += self.shared(&atlas.rgba8, atlas.rgba8.len(), &mut lease)?;
            }
            if let Some(glint) = &hand.item_glint {
                shared += self.shared(&glint.rgba8, glint.rgba8.len(), &mut lease)?;
            }
        }
        if let Some(hud) = &frame.hud {
            shared += self.shared(
                hud,
                std::mem::size_of::<render_model::PreparedUiFrame>(),
                &mut lease,
            )?;
            shared += self.allocation(&hud.input.vertices, &mut lease)?;
            shared += self.allocation(&hud.input.indices, &mut lease)?;
            shared += self.allocation(&hud.input.batches, &mut lease)?;
            for page in hud.input.textures.pages() {
                let pixels = page.shared_pixels();
                shared += self.allocation(&pixels, &mut lease)?;
            }
            if let Some(glint) = &hud.glint {
                let pixels = glint.shared_pixels();
                shared += self.allocation(&pixels, &mut lease)?;
            }
        }
        if let Some(atmosphere) = &frame.atmosphere {
            shared += self.shared(
                &atmosphere.assets,
                super::atmosphere::assets_bytes(&atmosphere.assets),
                &mut lease,
            )?;
            shared += self.allocation(&atmosphere.precipitation, &mut lease)?;
            shared += self.allocation(&atmosphere.occlusion_heights, &mut lease)?;
            shared += self.allocation(&atmosphere.lightning, &mut lease)?;
            shared += self.allocation(&atmosphere.overlays, &mut lease)?;
        }
        lease.charges.reserve(1);
        let owned = total
            .checked_sub(shared)
            .and_then(|bytes| {
                bytes.checked_add(lease.charges.capacity() * std::mem::size_of::<Arc<Charge>>())
            })
            .ok_or("Prepared visual retention accounting overflowed.")?;
        lease.charges.push(self.charge(owned, None)?);
        Ok((lease, owned))
    }

    /// Slice and structure allocations contribute their actual payload size without copying any data.
    fn allocation<T: ?Sized + Send + Sync + 'static>(
        &mut self,
        owner: &Arc<T>,
        lease: &mut Lease,
    ) -> Result<usize, String> {
        self.shared(owner, std::mem::size_of_val(&**owner), lease)
    }

    /// An immutable owner's byte charge remains live while queued samples reference that identity.
    fn shared<T: ?Sized + Send + Sync + 'static>(
        &mut self,
        owner: &Arc<T>,
        bytes: usize,
        lease: &mut Lease,
    ) -> Result<usize, String> {
        if bytes == 0 {
            return Ok(0);
        }
        let key = Allocation {
            kind: TypeId::of::<T>(),
            address: Arc::as_ptr(owner) as *const () as usize,
            bytes,
        };
        let charge = match self.allocations.get(&key).and_then(Weak::upgrade) {
            Some(charge) => charge,
            None => {
                let charge = self.charge(bytes, Some(Box::new(Arc::downgrade(owner))))?;
                self.allocations.insert(key, Arc::downgrade(&charge));
                charge
            }
        };
        lease.charges.push(charge);
        Ok(bytes)
    }

    /// All queued and currently encoded samples share the original track's byte allowance.
    fn charge(
        &self,
        bytes: usize,
        identity: Option<Box<dyn Send + Sync>>,
    ) -> Result<Arc<Charge>, String> {
        self.used.try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
            used.checked_add(bytes).filter(|&total| total <= MAX_QUEUED_BYTES)
        }).map_err(|used| format!("Prepared visual recording retains {used} bytes and needs {bytes} more; its limit is {MAX_QUEUED_BYTES} bytes."))?;
        Ok(Arc::new(Charge {
            used: Arc::clone(&self.used),
            bytes,
            _identity: identity,
        }))
    }
}

#[cfg(test)]
mod tests;
