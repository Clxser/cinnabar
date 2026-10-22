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

mod epochs;

pub(super) const MAX_QUEUED_BYTES: usize = 256 * 1024 * 1024;
pub(super) const MAX_FRAME_BYTES: usize = 192 * 1024 * 1024;
const SAMPLE_OVERHEAD: usize = 512;
const CHARGE_METADATA: usize = std::mem::size_of::<Charge>() + 2 * std::mem::size_of::<usize>();
const INDEX_METADATA: usize = std::mem::size_of::<(Allocation, Weak<Charge>)>();
const MIN_SHARED_CHARGE: usize =
    1 + CHARGE_METADATA + INDEX_METADATA + std::mem::size_of::<usize>();
const MAX_LIVE_KEYS: usize = MAX_QUEUED_BYTES / MIN_SHARED_CHARGE;
const MAX_STALE_KEYS: usize = epochs::MAX_DESCRIPTOR_BYTES / (INDEX_METADATA + CHARGE_METADATA);
const PRUNE_STEPS: usize = 16;

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
struct Allocation {
    kind: TypeId,
    address: usize,
    bytes: usize,
}

struct Charge {
    used: Arc<AtomicUsize>,
    bytes: usize,
    indexed: Option<Arc<AtomicUsize>>,
    // Weak ownership keeps the allocation address reserved until every accounting lease retires.
    _identity: Option<Box<dyn Send + Sync>>,
}

impl Drop for Charge {
    fn drop(&mut self) {
        self.used.fetch_sub(self.bytes, Ordering::AcqRel);
        if let Some(indexed) = &self.indexed {
            indexed.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

/// The encoder closure retains this lease until it has finished using its frozen sample.
#[derive(Default)]
pub(super) struct Lease {
    charges: Vec<Arc<Charge>>,
    groups: Vec<Arc<epochs::Group>>,
}

#[derive(Default)]
pub(super) struct Budget {
    used: Arc<AtomicUsize>,
    allocations: BTreeMap<Allocation, Weak<Charge>>,
    live_keys: Arc<AtomicUsize>,
    prune_after: Option<Allocation>,
    models: epochs::ItemCache,
    atlas: epochs::AtlasCache,
    #[cfg(test)]
    work: Work,
}

#[cfg(test)]
#[derive(Default)]
struct Work {
    models: usize,
    cells: usize,
    groups: usize,
    pins: usize,
    pruned: usize,
    logical: usize,
}

impl Budget {
    /// Charges shared immutable catalogs once and the remaining draw data for every queued sample.
    pub(super) fn retain(&mut self, frame: &FrozenVisuals) -> Result<(Lease, usize), String> {
        self.prune();
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
            shared += epochs::atlas(self, &scene.atlas, &mut lease)?;
        } else {
            self.atlas.clear();
        }
        if let Some(items) = &frame.items {
            shared += epochs::items(self, &items.models, &mut lease)?;
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
        } else {
            self.models.clear();
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
        let base = std::mem::size_of::<FrozenVisuals>()
            + SAMPLE_OVERHEAD
            + frame
                .hand
                .as_ref()
                .map_or(0, |_| std::mem::size_of::<super::PreparedHand>())
            + frame.atmosphere.as_ref().map_or(0, |_| {
                std::mem::size_of::<super::atmosphere::AtmosphereVisuals>()
            });
        let _total = shared
            .checked_add(base)
            .filter(|&bytes| bytes <= MAX_FRAME_BYTES)
            .ok_or("The prepared visual frame exceeds its memory budget.")?;
        #[cfg(test)]
        {
            self.work.logical = _total;
        }
        lease.charges.reserve(1);
        let owned = base
            .checked_add(lease.charges.capacity() * std::mem::size_of::<Arc<Charge>>())
            .and_then(|bytes| {
                bytes.checked_add(
                    lease.groups.capacity() * std::mem::size_of::<Arc<epochs::Group>>(),
                )
            })
            .and_then(|bytes| bytes.checked_add(CHARGE_METADATA))
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
        let key = Allocation::new(owner, bytes);
        let charge = self.shared_key(key, std::mem::size_of::<Weak<T>>(), || {
            Box::new(Arc::downgrade(owner))
        })?;
        lease.charges.push(charge);
        Ok(bytes)
    }

    /// Known immutable layouts need a child weak identity only when its previous queue charge expired.
    fn shared_key(
        &mut self,
        key: Allocation,
        weak_bytes: usize,
        pin: impl FnOnce() -> Box<dyn Send + Sync>,
    ) -> Result<Arc<Charge>, String> {
        if let Some(charge) = self.allocations.get(&key).and_then(Weak::upgrade) {
            return Ok(charge);
        }
        let bytes = key
            .bytes
            .checked_add(CHARGE_METADATA + INDEX_METADATA + weak_bytes)
            .ok_or("Prepared visual retention accounting overflowed.")?;
        #[cfg(test)]
        {
            self.work.pins += 1;
        }
        let charge = self.charge(bytes, Some(pin()))?;
        self.allocations.insert(key, Arc::downgrade(&charge));
        self.bound_index(MAX_STALE_KEYS);
        Ok(charge)
    }

    /// Dead keys have a finite independent allowance even when a busy epoch admits many new children.
    fn bound_index(&mut self, stale_allowance: usize) {
        if self.allocations.len()
            > self.live_keys.load(Ordering::Acquire).min(MAX_LIVE_KEYS) + stale_allowance
        {
            self.allocations
                .retain(|_, charge| charge.strong_count() != 0);
            self.prune_after = None;
        }
    }

    /// A small ordered sweep reclaims dead keys; the queue-derived hard threshold bounds stale metadata.
    fn prune(&mut self) {
        use std::ops::Bound::{Excluded, Unbounded};
        let start = self.prune_after.map_or(Unbounded, Excluded);
        let mut keys = [None; PRUNE_STEPS];
        for (slot, (&key, _)) in keys
            .iter_mut()
            .zip(self.allocations.range((start, Unbounded)))
        {
            *slot = Some(key);
        }
        if keys[0].is_none() {
            self.prune_after = None;
            for (slot, &key) in keys.iter_mut().zip(self.allocations.keys()) {
                *slot = Some(key);
            }
        }
        for key in keys.into_iter().flatten() {
            self.prune_after = Some(key);
            #[cfg(test)]
            {
                self.work.pruned += 1;
            }
            if self
                .allocations
                .get(&key)
                .is_some_and(|charge| charge.strong_count() == 0)
            {
                self.allocations.remove(&key);
            }
        }
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
        let indexed = identity.as_ref().map(|_| Arc::clone(&self.live_keys));
        if let Some(indexed) = &indexed {
            indexed.fetch_add(1, Ordering::AcqRel);
        }
        Ok(Arc::new(Charge {
            used: Arc::clone(&self.used),
            bytes,
            indexed,
            _identity: identity,
        }))
    }
}

impl Allocation {
    /// Type, allocation address and payload size identify one immutable Arc allocation, never its contents.
    fn new<T: ?Sized + 'static>(owner: &Arc<T>, bytes: usize) -> Self {
        Self {
            kind: TypeId::of::<T>(),
            address: Arc::as_ptr(owner) as *const () as usize,
            bytes,
        }
    }
}

#[cfg(test)]
mod tests;
