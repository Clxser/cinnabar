//! Immutable catalog layouts cache projections, never child pixel ownership or live queue charges.

use super::*;
use render_model::{DroppedItemModel, DroppedItemSprite, NametagAtlasRect};

/// Both caches fit in a small fraction of one admitted frame; larger layouts use ordinary accounting.
pub(super) const MAX_DESCRIPTOR_BYTES: usize = MAX_FRAME_BYTES / 32;

pub(super) type ItemCache = Cache<[DroppedItemModel], ItemChild>;
pub(super) type AtlasCache = Cache<[NametagAtlasRect], AtlasChild>;

pub(super) struct Group {
    _charges: Vec<Arc<Charge>>,
    _metadata: Arc<Charge>,
}

/// The aggregate lease owns one vector of child charges, one header, and its metadata charge.
pub(super) fn group_metadata(slots: usize) -> usize {
    std::mem::size_of::<Group>()
        + 2 * std::mem::size_of::<usize>()
        + CHARGE_METADATA
        + slots * std::mem::size_of::<Arc<Charge>>()
}

pub(super) struct Cache<T: ?Sized, C> {
    current: Option<Cached<T, C>>,
}

impl<T: ?Sized, C> Default for Cache<T, C> {
    fn default() -> Self {
        Self { current: None }
    }
}

struct Cached<T: ?Sized, C> {
    owner: Weak<T>,
    layout: Layout<C>,
    active: Weak<Group>,
}

struct Layout<C> {
    entries: Vec<Entry<C>>,
    logical: usize,
    root_bytes: usize,
}

struct Entry<C> {
    key: Allocation,
    weak_bytes: usize,
    child: C,
}

#[derive(Clone, Copy)]
pub(super) enum ItemChild {
    Root,
    Sprite(usize),
    Face(usize, usize),
    Materials(usize),
    Quads(usize),
    MaterialSprite(usize, usize),
}

#[derive(Clone, Copy)]
pub(super) enum AtlasChild {
    Root,
    Cell(usize),
}

enum Owner<'a> {
    Models(&'a Arc<[DroppedItemModel]>),
    Pixels(&'a Arc<[u8]>),
    Materials(&'a Arc<[(DroppedItemSprite, u32)]>),
    Quads(&'a Arc<[assets::ModelQuad]>),
    Atlas(&'a Arc<[NametagAtlasRect]>),
}

impl Owner<'_> {
    /// Describing the direct allocation reads its slice length without visiting any texel or child.
    fn describe(&self) -> (Allocation, usize) {
        /// Typed Arc metadata supplies its allocation key and the exact weak identity storage size.
        fn describe<T: ?Sized + 'static>(owner: &Arc<T>) -> (Allocation, usize) {
            (
                Allocation::new(owner, std::mem::size_of_val(&**owner)),
                std::mem::size_of::<Weak<T>>(),
            )
        }
        match self {
            Self::Models(owner) => describe(owner),
            Self::Pixels(owner) => describe(owner),
            Self::Materials(owner) => describe(owner),
            Self::Quads(owner) => describe(owner),
            Self::Atlas(owner) => describe(owner),
        }
    }

    /// Only a live queue charge pins the child's control block against allocation-address reuse.
    fn pin(self) -> Box<dyn Send + Sync> {
        match self {
            Self::Models(owner) => Box::new(Arc::downgrade(owner)),
            Self::Pixels(owner) => Box::new(Arc::downgrade(owner)),
            Self::Materials(owner) => Box::new(Arc::downgrade(owner)),
            Self::Quads(owner) => Box::new(Arc::downgrade(owner)),
            Self::Atlas(owner) => Box::new(Arc::downgrade(owner)),
        }
    }
}

impl<C> Layout<C> {
    /// The root's weak reservation can retain its backing allocation, so it is included in the cap.
    fn new<T: ?Sized>(root: &Arc<T>) -> Option<Self> {
        let layout = Self {
            entries: Vec::new(),
            logical: 0,
            root_bytes: std::mem::size_of_val(&**root),
        };
        (layout.metadata::<T>() <= MAX_DESCRIPTOR_BYTES).then_some(layout)
    }

    /// Cache limits never reject valid data: reaching this cap selects the uncached path instead.
    fn push<T: ?Sized>(&mut self, owner: Owner<'_>, child: C) -> Result<bool, String> {
        let (key, weak_bytes) = owner.describe();
        if key.bytes == 0 {
            return Ok(true);
        }
        self.logical = self
            .logical
            .checked_add(key.bytes)
            .filter(|&bytes| bytes <= MAX_FRAME_BYTES)
            .ok_or("The prepared visual frame exceeds its memory budget.")?;
        if self.entries.len() == self.entries.capacity() {
            let additional = self.entries.capacity().max(8);
            let next = self
                .entries
                .capacity()
                .checked_add(additional)
                .and_then(|capacity| capacity.checked_mul(std::mem::size_of::<Entry<C>>()))
                .and_then(|bytes| {
                    bytes.checked_add(
                        self.root_bytes
                            + std::mem::size_of::<Cached<T, C>>()
                            + std::mem::size_of::<Group>()
                            + 4 * std::mem::size_of::<usize>(),
                    )
                });
            if next.is_none_or(|bytes| bytes > MAX_DESCRIPTOR_BYTES)
                || self.entries.try_reserve(additional).is_err()
            {
                return Ok(false);
            }
            if self.metadata::<T>() > MAX_DESCRIPTOR_BYTES {
                return Ok(false);
            }
        }
        self.entries.push(Entry {
            key,
            weak_bytes,
            child,
        });
        Ok(true)
    }

    /// Includes the root weak/control reservation, inline cache data and all reserved descriptor slots.
    fn metadata<T: ?Sized>(&self) -> usize {
        self.root_bytes
            + std::mem::size_of::<Cached<T, C>>()
            + std::mem::size_of::<Group>()
            + 4 * std::mem::size_of::<usize>()
            + self.entries.capacity() * std::mem::size_of::<Entry<C>>()
    }
}

impl<T: ?Sized + 'static, C: Copy> Cache<T, C> {
    /// Losing the source channel releases stale descriptors without retaining an absent catalog epoch.
    pub(super) fn clear(&mut self) {
        self.current = None;
    }

    /// Regression instrumentation includes retained weak root/group storage, not just live entry length.
    #[cfg(test)]
    pub(super) fn metadata_bytes(&self) -> usize {
        self.current
            .as_ref()
            .map_or(0, |cached| cached.layout.metadata::<T>())
    }

    /// An active epoch attaches one shared group; cold reactivation visits only cached direct projections.
    fn retain(
        &mut self,
        budget: &mut Budget,
        owner: &Arc<T>,
        lease: &mut Lease,
        build: impl FnOnce(&mut Budget, &Arc<T>) -> Result<Option<Layout<C>>, String>,
        project: for<'a> fn(&'a Arc<T>, C) -> Owner<'a>,
        fallback: impl FnOnce(&mut Budget, &Arc<T>, &mut Lease) -> Result<usize, String>,
    ) -> Result<usize, String> {
        let matches = self
            .current
            .as_ref()
            .is_some_and(|cached| cached.owner.ptr_eq(&Arc::downgrade(owner)));
        if !matches {
            // Retire old metadata before allocating its replacement, even if admission later fails.
            self.current = None;
            self.current = build(budget, owner)?.map(|layout| Cached {
                owner: Arc::downgrade(owner),
                layout,
                active: Weak::new(),
            });
        }
        let Some(cached) = &mut self.current else {
            return fallback(budget, owner, lease);
        };
        if cached.layout.logical == 0 {
            return Ok(0);
        }
        let group = if let Some(active) = cached.active.upgrade() {
            active
        } else {
            let mut charges = Vec::with_capacity(cached.layout.entries.len());
            for entry in &cached.layout.entries {
                charges.push(budget.shared_key(entry.key, entry.weak_bytes, || {
                    project(owner, entry.child).pin()
                })?);
            }
            let bytes = group_metadata(charges.capacity());
            let metadata = budget.charge(bytes, None)?;
            let group = Arc::new(Group {
                _charges: charges,
                _metadata: metadata,
            });
            #[cfg(test)]
            {
                budget.work.groups += 1;
            }
            cached.active = Arc::downgrade(&group);
            group
        };
        lease.groups.push(group);
        Ok(cached.layout.logical)
    }
}

/// Immutable root identity permits direct projections only; a changed child necessarily changes the root.
fn item_owner(models: &Arc<[DroppedItemModel]>, child: ItemChild) -> Owner<'_> {
    match child {
        ItemChild::Root => Owner::Models(models),
        ItemChild::Sprite(index) => match &models[index] {
            DroppedItemModel::Sprite(sprite) | DroppedItemModel::NativeSprite(sprite) => {
                Owner::Pixels(&sprite.rgba8)
            }
            _ => unreachable!("immutable item descriptor variant"),
        },
        ItemChild::Face(index, face) => match &models[index] {
            DroppedItemModel::Cube(cube) => Owner::Pixels(&cube.faces[face]),
            _ => unreachable!("immutable cube descriptor variant"),
        },
        ItemChild::Materials(index) => match &models[index] {
            DroppedItemModel::Block(block) => Owner::Materials(&block.materials),
            _ => unreachable!("immutable block descriptor variant"),
        },
        ItemChild::Quads(index) => match &models[index] {
            DroppedItemModel::Block(block) => Owner::Quads(&block.quads),
            _ => unreachable!("immutable block descriptor variant"),
        },
        ItemChild::MaterialSprite(index, material) => match &models[index] {
            DroppedItemModel::Block(block) => Owner::Pixels(&block.materials[material].0.rgba8),
            _ => unreachable!("immutable material descriptor variant"),
        },
    }
}

/// Every model variant enumerates its owned allocations once when the catalog identity changes.
fn walk_items(
    budget: &mut Budget,
    models: &Arc<[DroppedItemModel]>,
    mut visit: impl FnMut(&mut Budget, ItemChild) -> Result<bool, String>,
) -> Result<bool, String> {
    if !visit(budget, ItemChild::Root)? {
        return Ok(false);
    }
    for (index, model) in models.iter().enumerate() {
        #[cfg(test)]
        {
            budget.work.models += 1;
        }
        let keep = match model {
            DroppedItemModel::Vacant => true,
            DroppedItemModel::Sprite(_) | DroppedItemModel::NativeSprite(_) => {
                visit(budget, ItemChild::Sprite(index))?
            }
            DroppedItemModel::Cube(cube) => {
                let mut keep = true;
                for face in 0..cube.faces.len() {
                    if !visit(budget, ItemChild::Face(index, face))? {
                        keep = false;
                        break;
                    }
                }
                keep
            }
            DroppedItemModel::Block(block) => {
                if !visit(budget, ItemChild::Materials(index))?
                    || !visit(budget, ItemChild::Quads(index))?
                {
                    false
                } else {
                    let mut keep = true;
                    for material in 0..block.materials.len() {
                        if !visit(budget, ItemChild::MaterialSprite(index, material))? {
                            keep = false;
                            break;
                        }
                    }
                    keep
                }
            }
        };
        if !keep {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Per-frame draw changes do not invalidate the immutable item catalog descriptor.
pub(super) fn items(
    budget: &mut Budget,
    models: &Arc<[DroppedItemModel]>,
    lease: &mut Lease,
) -> Result<usize, String> {
    let mut cache = std::mem::take(&mut budget.models);
    let result = cache.retain(
        budget,
        models,
        lease,
        |budget, models| {
            let Some(mut layout) = Layout::new(models) else {
                return Ok(None);
            };
            let complete = walk_items(budget, models, |_, child| {
                layout.push::<[DroppedItemModel]>(item_owner(models, child), child)
            })?;
            Ok(complete.then_some(layout))
        },
        item_owner,
        |budget, models, lease| {
            let mut bytes = 0_usize;
            walk_items(budget, models, |budget, child| {
                let owner = item_owner(models, child);
                let (key, weak_bytes) = owner.describe();
                if key.bytes != 0 {
                    bytes = bytes
                        .checked_add(key.bytes)
                        .ok_or("Prepared visual retention accounting overflowed.")?;
                    lease
                        .charges
                        .push(budget.shared_key(key, weak_bytes, || owner.pin())?);
                }
                Ok(true)
            })?;
            Ok(bytes)
        },
    );
    budget.models = cache;
    result
}

/// Atlas projections retain their child identity without hashing or reading pixel contents.
fn atlas_owner(atlas: &Arc<[NametagAtlasRect]>, child: AtlasChild) -> Owner<'_> {
    match child {
        AtlasChild::Root => Owner::Atlas(atlas),
        AtlasChild::Cell(index) => Owner::Pixels(&atlas[index].rgba8),
    }
}

/// New tag positions reuse the atlas descriptor until the immutable cell catalog itself changes.
pub(super) fn atlas(
    budget: &mut Budget,
    atlas: &Arc<[NametagAtlasRect]>,
    lease: &mut Lease,
) -> Result<usize, String> {
    let mut cache = std::mem::take(&mut budget.atlas);
    let result = cache.retain(
        budget,
        atlas,
        lease,
        |_budget, atlas| {
            let Some(mut layout) = Layout::new(atlas) else {
                return Ok(None);
            };
            if !layout.push::<[NametagAtlasRect]>(Owner::Atlas(atlas), AtlasChild::Root)? {
                return Ok(None);
            }
            for index in 0..atlas.len() {
                #[cfg(test)]
                {
                    _budget.work.cells += 1;
                }
                let child = AtlasChild::Cell(index);
                if !layout.push::<[NametagAtlasRect]>(atlas_owner(atlas, child), child)? {
                    return Ok(None);
                }
            }
            Ok(Some(layout))
        },
        atlas_owner,
        |budget, atlas, lease| {
            let mut bytes = budget.allocation(atlas, lease)?;
            for cell in atlas.iter() {
                #[cfg(test)]
                {
                    budget.work.cells += 1;
                }
                bytes += budget.allocation(&cell.rgba8, lease)?;
            }
            Ok(bytes)
        },
    );
    budget.atlas = cache;
    result
}
