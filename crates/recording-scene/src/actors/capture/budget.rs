//! Shared immutable allocations count once while any queued actor epoch retains them.

use std::{
    any::TypeId,
    collections::BTreeMap,
    sync::{
        Arc, Weak,
        atomic::{AtomicUsize, Ordering},
    },
};

use render_model::{
    ActorArtworkPages, ActorRigGeometrySpan, ActorRigVertexSegments, ActorSkinResidency,
};

use super::super::{appearance, geometry};

const MAX_PINNED_BYTES: usize = geometry::MAX_BYTES + appearance::MAX_BYTES;

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
struct Allocation {
    kind: TypeId,
    address: usize,
    bytes: usize,
}

struct Charge {
    used: Arc<AtomicUsize>,
    bytes: usize,
}

impl Drop for Charge {
    fn drop(&mut self) {
        self.used.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

/// Must be dropped before its owning snapshot's asset fields, preventing allocation-address reuse.
pub(super) struct Lease {
    charges: Vec<Arc<Charge>>,
}

#[derive(Default)]
pub(super) struct Budget {
    used: Arc<AtomicUsize>,
    allocations: BTreeMap<Allocation, Weak<Charge>>,
}

impl Budget {
    /// Accounts for catalog tables and vertex pages shared across geometry revisions.
    pub(super) fn geometry(
        &mut self,
        vertices: &ActorRigVertexSegments,
        spans: &Arc<[ActorRigGeometrySpan]>,
    ) -> Result<Lease, String> {
        let mut lease = self.begin();
        self.pin(&vertices.segments, &mut lease)?;
        self.pin(&vertices.offsets, &mut lease)?;
        self.pin(spans, &mut lease)?;
        for page in vertices.segments.iter() {
            self.pin(page, &mut lease)?;
        }
        Ok(lease)
    }

    /// Charges changed skin tables independently of unchanged source rasters and artwork pages.
    pub(super) fn appearance(
        &mut self,
        skins: &Arc<ActorSkinResidency>,
        artwork: &Arc<ActorArtworkPages>,
    ) -> Result<Lease, String> {
        let mut lease = self.begin();
        self.pin(skins, &mut lease)?;
        self.pin(artwork, &mut lease)?;
        for class in &skins.classes {
            self.pin(class, &mut lease)?;
            for resident in class.iter().flatten() {
                self.pin(resident.skin.pixels(), &mut lease)?;
                self.pin(&resident.texels, &mut lease)?;
            }
        }
        self.pin(&artwork.pages, &mut lease)?;
        for page in artwork.pages.iter() {
            self.pin(&page.rgba8, &mut lease)?;
        }
        for raster in [artwork.actor_glint.as_ref(), artwork.item_glint.as_ref()]
            .into_iter()
            .flatten()
        {
            self.pin(&raster.rgba8, &mut lease)?;
        }
        Ok(lease)
    }

    /// Removes expired identities once per asset preparation, without retaining obsolete assets.
    fn begin(&mut self) -> Lease {
        self.allocations
            .retain(|_, charge| charge.strong_count() > 0);
        Lease {
            charges: Vec::new(),
        }
    }

    /// A live charge has an owning snapshot holding this allocation; empty slices need no charge.
    fn pin<T: ?Sized + 'static>(
        &mut self,
        asset: &Arc<T>,
        lease: &mut Lease,
    ) -> Result<(), String> {
        let bytes = std::mem::size_of_val(&**asset);
        if bytes == 0 {
            return Ok(());
        }
        let key = Allocation {
            kind: TypeId::of::<T>(),
            address: Arc::as_ptr(asset) as *const () as usize,
            bytes,
        };
        if let Some(charge) = self.allocations.get(&key).and_then(Weak::upgrade) {
            lease.charges.push(charge);
            return Ok(());
        }
        self.used
            .try_update(Ordering::AcqRel, Ordering::Acquire, |old| {
                old.checked_add(bytes).filter(|&total| total <= MAX_PINNED_BYTES)
            })
            .map_err(|pinned| format!(
                "Recorded actor assets exceeded their pinned memory budget: {pinned} bytes retained, {bytes} additional bytes requested, limit {MAX_PINNED_BYTES}."
            ))?;
        let charge = Arc::new(Charge {
            used: Arc::clone(&self.used),
            bytes,
        });
        self.allocations.insert(key, Arc::downgrade(&charge));
        lease.charges.push(charge);
        Ok(())
    }
}
