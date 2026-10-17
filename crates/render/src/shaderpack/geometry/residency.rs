//! Where each resident section's expanded faces live: vertex pages within the memory budget,
//! the expansion queue, and the origin that positions are relative to.

use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::Range;

use bevy::math::{DVec3, IVec3};
use bevy::prelude::Entity;

use super::allocator::FaceAllocator;
use super::jobs::SectionPlan;
use super::vertex::IRIS_VERTEX_STRIDE;

/// Bytes of one expanded face: four vertices.
pub(crate) const FACE_BYTES: u64 = 4 * IRIS_VERTEX_STRIDE;
/// Faces a new vertex page starts with before it grows.
pub(crate) const INITIAL_PAGE_FACES: u32 = 1 << 16;
/// Grid the geometry origin snaps to, in blocks.
pub(crate) const ORIGIN_GRID: i32 = 1024;
/// Camera distance from the origin, in blocks, past which the origin moves and every section
/// re-expands. f32 positions within this range keep sub-millimetre precision.
pub(crate) const ORIGIN_REBASE_DISTANCE: f64 = 4096.0;

/// A resident section's faces: their page, the range they occupy and each category's range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResidentFaces {
    pub(crate) page: usize,
    pub(crate) faces: Range<u32>,
    pub(crate) categories: [Range<u32>; 3],
    /// Section origin in blocks.
    pub(crate) origin: [i32; 3],
    /// Origin epoch the positions were written against; stale faces are not drawn.
    pub(crate) epoch: u64,
}

/// Face space of one vertex page. Pages never exceed one storage-buffer binding, since the
/// kernels bind a page whole.
#[derive(Debug, Default)]
pub(crate) struct PageSpace {
    pub(crate) faces: FaceAllocator,
}

/// Where a planned section was placed, and the page growth that placement needs.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Placement {
    pub(crate) page: usize,
    pub(crate) faces: Range<u32>,
    /// The page's capacity in faces before this placement grew it, if it did.
    pub(crate) grown_from: Option<u32>,
}

/// CPU bookkeeping of the expanded arena.
#[derive(Debug)]
pub(crate) struct Residency {
    pub(crate) pages: Vec<PageSpace>,
    pub(crate) sections: HashMap<Entity, ResidentFaces>,
    queue: VecDeque<Entity>,
    queued: HashSet<Entity>,
    /// Faces one page may hold.
    page_faces: u32,
    /// Faces all pages together may hold.
    budget_faces: u64,
    pub(crate) origin: Option<IVec3>,
    pub(crate) epoch: u64,
}

impl Residency {
    /// `binding_bytes` is the largest buffer a kernel may bind; `budget_bytes` the memory budget.
    pub(crate) fn new(binding_bytes: u64, budget_bytes: u64) -> Self {
        let page_faces = (binding_bytes / FACE_BYTES).min(u64::from(u32::MAX / 4)) as u32;
        Self {
            pages: Vec::new(),
            sections: HashMap::new(),
            queue: VecDeque::new(),
            queued: HashSet::new(),
            page_faces,
            budget_faces: budget_bytes / FACE_BYTES,
            origin: None,
            epoch: 0,
        }
    }

    /// Changes the memory budget; pages already open stay until they empty.
    pub(crate) fn set_budget(&mut self, budget_bytes: u64) {
        self.budget_faces = budget_bytes / FACE_BYTES;
    }

    fn capacity_faces(&self) -> u64 {
        self.pages
            .iter()
            .map(|page| u64::from(page.faces.capacity()))
            .sum()
    }

    /// Queues `entity` for (re-)expansion once.
    pub(crate) fn enqueue(&mut self, entity: Entity) {
        if self.queued.insert(entity) {
            self.queue.push_back(entity);
        }
    }

    /// Queues every resident section, as after an origin, table or atlas change.
    pub(crate) fn enqueue_all(&mut self, entities: impl IntoIterator<Item = Entity>) {
        for entity in entities {
            self.enqueue(entity);
        }
    }

    pub(crate) fn queue_len(&self) -> usize {
        self.queue.len()
    }

    /// Next queued section.
    pub(crate) fn pop(&mut self) -> Option<Entity> {
        let entity = self.queue.pop_front()?;
        self.queued.remove(&entity);
        Some(entity)
    }

    /// Re-queues `entity` at the back after its placement did not fit.
    pub(crate) fn defer(&mut self, entity: Entity) {
        self.enqueue(entity);
    }

    /// Frees a removed section's faces.
    pub(crate) fn remove(&mut self, entity: Entity) {
        if let Some(resident) = self.sections.remove(&entity) {
            self.pages[resident.page].faces.release(resident.faces);
        }
        if self.queued.remove(&entity) {
            self.queue.retain(|queued| *queued != entity);
        }
    }

    /// Moves the origin when `camera` has left its range; returns whether it moved, in which
    /// case every resident section needs expanding again.
    pub(crate) fn follow_camera(&mut self, camera: DVec3) -> bool {
        let snapped = (camera / f64::from(ORIGIN_GRID)).round() * f64::from(ORIGIN_GRID);
        let snapped = IVec3::new(snapped.x as i32, snapped.y as i32, snapped.z as i32);
        match self.origin {
            Some(origin) if camera.distance(origin.as_dvec3()) <= ORIGIN_REBASE_DISTANCE => false,
            _ => {
                self.origin = Some(snapped);
                self.epoch += 1;
                true
            }
        }
    }

    /// Places `plan` for `entity`, replacing its previous faces, growing a page or adding one
    /// within the budget. `None` when it does not fit; the section then keeps its old faces.
    pub(crate) fn place(&mut self, entity: Entity, plan: &SectionPlan) -> Option<Placement> {
        if plan.faces > self.page_faces {
            return None;
        }
        let previous = self.sections.remove(&entity);
        if let Some(previous) = &previous {
            self.pages[previous.page]
                .faces
                .release(previous.faces.clone());
        }
        let placement = self.allocate(plan.faces);
        if placement.is_none()
            && let Some(previous) = previous
        {
            // Keep drawing the old faces until a later frame finds room.
            let reclaimed = self.pages[previous.page]
                .faces
                .allocate_at(previous.faces.clone());
            debug_assert!(reclaimed, "released faces are free");
            self.sections.insert(entity, previous);
        }
        placement
    }

    fn allocate(&mut self, faces: u32) -> Option<Placement> {
        for (page, space) in self.pages.iter_mut().enumerate() {
            if let Some(range) = space.faces.allocate(faces) {
                return Some(Placement {
                    page,
                    faces: range,
                    grown_from: None,
                });
            }
        }
        // Grow the last page, doubling, before opening another.
        let budget_left = self.budget_faces.saturating_sub(self.capacity_faces());
        if let Some(page) = self.pages.len().checked_sub(1) {
            let current = self.pages[page].faces.capacity();
            let wanted = current
                .saturating_mul(2)
                .max(current + faces)
                .min(self.page_faces);
            let grown = u64::from(wanted - current).min(budget_left) as u32 + current;
            if grown - current + self.pages[page].faces.tail_free() >= faces {
                self.pages[page].faces.grow(grown);
                if let Some(range) = self.pages[page].faces.allocate(faces) {
                    return Some(Placement {
                        page,
                        faces: range,
                        grown_from: Some(current),
                    });
                }
            }
        }
        let budget_left = self.budget_faces.saturating_sub(self.capacity_faces());
        let capacity =
            u64::from(INITIAL_PAGE_FACES.max(faces).min(self.page_faces)).min(budget_left) as u32;
        if capacity < faces || faces == 0 {
            return None;
        }
        let mut space = PageSpace::default();
        space.faces.grow(capacity);
        let range = space.faces.allocate(faces)?;
        self.pages.push(space);
        Some(Placement {
            page: self.pages.len() - 1,
            faces: range,
            grown_from: Some(0),
        })
    }

    /// Records `entity`'s placed plan.
    pub(crate) fn commit(
        &mut self,
        entity: Entity,
        placement: &Placement,
        plan: &SectionPlan,
        origin: [i32; 3],
    ) {
        self.sections.insert(
            entity,
            ResidentFaces {
                page: placement.page,
                faces: placement.faces.clone(),
                categories: plan.categories.clone(),
                origin,
                epoch: self.epoch,
            },
        );
    }

    /// Drops every page and section, as when the mode turns off.
    pub(crate) fn clear(&mut self) {
        self.pages.clear();
        self.sections.clear();
        self.queue.clear();
        self.queued.clear();
        self.origin = None;
    }
}

#[cfg(test)]
#[path = "residency_tests.rs"]
mod tests;
