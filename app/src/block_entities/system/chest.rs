//! Double chests resolved from each chest's own pairing claim and its loaded neighbours.
//!
//! A leading chest's record initiates pairing; the partner draws
//! nothing once the lead pairs with it, whatever its own record says. Pairing waits until
//! both actors are loaded on compatible blocks, so either arrival order converges.

use bevy::platform::collections::HashMap;
use render_model::{ChestModel, ChestPair, ChestVariant, Facing};
use world::{BlockEntityKey, ChunkKey, ChunkStore};

use super::{
    BlockEntityRuntime, PhysicsCollisionRegistries, Template, block_info,
    columns::{ColumnEntity, ColumnScan},
    describe,
};

/// Where the chest templates around the column being drawn come from.
pub(super) struct Neighbourhood<'a> {
    /// The column being drawn, whose scan is held outside `scans` meanwhile.
    pub(super) column: ChunkKey,
    pub(super) entities: &'a [ColumnEntity],
    pub(super) scans: &'a HashMap<ChunkKey, ColumnScan>,
    pub(super) store: &'a ChunkStore,
    pub(super) collisions: &'a PhysicsCollisionRegistries,
    pub(super) mode: assets::NetworkIdMode,
}

impl Neighbourhood<'_> {
    /// Reads current neighbour templates even when their column has not been rescanned yet.
    pub(super) fn chest(
        &self,
        key: BlockEntityKey,
        runtime: &mut BlockEntityRuntime,
    ) -> Option<ChestModel> {
        self.chest_or_else(key, || {
            stored_template(key, self.store, runtime, self.collisions, self.mode)
        })
    }

    /// Reuses a current scan, calling `fallback` only for an absent or stale neighbour scan.
    fn chest_or_else(
        &self,
        key: BlockEntityKey,
        fallback: impl FnOnce() -> Option<Template>,
    ) -> Option<ChestModel> {
        let entities = if key.chunk() == self.column {
            Some(self.entities)
        } else {
            self.scans
                .get(&key.chunk())
                .filter(|scan| {
                    self.store
                        .chunk(key.chunk())
                        .is_some_and(|chunk| scan.is_current(chunk))
                })
                .map(|scan| scan.entities.as_slice())
        };
        let stored;
        let template = match entities {
            Some(entities) => {
                let index = entities
                    .binary_search_by(|entity| entity.key.cmp(&key))
                    .ok()?;
                entities[index].template.as_ref()
            }
            None => {
                stored = fallback();
                stored.as_ref()
            }
        };
        match template? {
            &Template::Chest(model) => Some(model),
            _ => None,
        }
    }
}

/// `model` with its pair resolved: its own claim when that claim holds, a follower when a
/// neighbour's claim on it holds, and single otherwise. `chest_at` reads loaded chest
/// templates, whose `pair` is the claim their own record makes.
pub(super) fn paired_model(
    mut model: ChestModel,
    key: BlockEntityKey,
    mut chest_at: impl FnMut(BlockEntityKey) -> Option<ChestModel>,
) -> ChestModel {
    if model.variant == ChestVariant::Ender {
        return model;
    }
    if let Some(partner) = claim(model)
        && holds(key, model, partner, &mut chest_at)
    {
        return model;
    }
    let led = neighbours(key, model.facing)
        .into_iter()
        .flatten()
        .any(|at| {
            chest_at(at).is_some_and(|other| {
                claim(other) == Some(key.position())
                    && holds(at, other, key.position(), &mut chest_at)
            })
        });
    model.pair = if led {
        ChestPair::Follower
    } else {
        ChestPair::Single
    };
    model
}

/// Whether `model` at `lead` draws the double chest over `partner`.
///
/// The partner must be a loaded, compatible chest beside the lead along the pairing axis.
/// Vanilla settles competing claims by actor tick order; until that order is matched, a
/// partner keeps its own valid claim, mutual claims go to the lower position, and the lower
/// of two claimants wins a shared partner.
fn holds(
    lead: BlockEntityKey,
    model: ChestModel,
    partner: [i32; 3],
    chest_at: &mut impl FnMut(BlockEntityKey) -> Option<ChestModel>,
) -> bool {
    let Some(step) = [-1, 1]
        .into_iter()
        .find(|&step| offset(lead, model.facing, step).is_some_and(|at| at.position() == partner))
    else {
        return false;
    };
    let partner = BlockEntityKey::new(lead.dimension, partner[0], partner[1], partner[2]);
    let Some(other) = chest_at(partner).filter(|other| compatible(model, *other)) else {
        return false;
    };
    let beyond = offset(partner, model.facing, step);
    let rival = beyond.and_then(|at| {
        chest_at(at)
            .filter(|chest| compatible(other, *chest))
            .map(|chest| (at, chest))
    });
    match claim(other) {
        Some(back) if back == lead.position() => lead < partner,
        Some(ahead) if rival.is_some_and(|(at, _)| at.position() == ahead) => false,
        _ => {
            !rival.is_some_and(|(at, chest)| claim(chest) == Some(partner.position()) && at < lead)
        }
    }
}

/// The partner a chest's own record names while leading.
fn claim(model: ChestModel) -> Option<[i32; 3]> {
    match model.pair {
        ChestPair::Lead { partner } => Some(partner),
        _ => None,
    }
}

/// Chests pair only with the same facing and kind; copper chests of any age pair together.
fn compatible(a: ChestModel, b: ChestModel) -> bool {
    a.facing == b.facing
        && a.variant != ChestVariant::Ender
        && std::mem::discriminant(&a.variant) == std::mem::discriminant(&b.variant)
}

/// The two positions a chest facing `facing` can pair with.
fn neighbours(key: BlockEntityKey, facing: Facing) -> [Option<BlockEntityKey>; 2] {
    [-1, 1].map(|step| offset(key, facing, step))
}

/// `key` moved `step` blocks along the axis across a chest's front.
fn offset(key: BlockEntityKey, facing: Facing, step: i32) -> Option<BlockEntityKey> {
    let mut moved = key;
    match facing {
        Facing::North | Facing::South => moved.x = key.x.checked_add(step)?,
        Facing::East | Facing::West => moved.z = key.z.checked_add(step)?,
    }
    Some(moved)
}

/// Describes the chest at `key` when both its block and block actor are loaded, reusing the
/// record's shared parse so repeated lookups never parse it again.
fn stored_template(
    key: BlockEntityKey,
    store: &ChunkStore,
    runtime: &mut BlockEntityRuntime,
    collisions: &PhysicsCollisionRegistries,
    mode: assets::NetworkIdMode,
) -> Option<Template> {
    let [x, y, z] = key.position();
    let id = store.sub_chunk_ref(key.sub_chunk())?.runtime_id(
        0,
        (x & 15) as u8,
        (y & 15) as u8,
        (z & 15) as u8,
    )?;
    let info = block_info(runtime, collisions, mode, id)?;
    if info.backing?.entity.renderer != assets::facts::BlockEntityRenderer::Chest {
        return None;
    }
    let nbt = store.block_entity(key)?;
    describe(
        nbt.id()?,
        info.backing?,
        &info.name,
        &info.state,
        nbt.parsed()?.as_ref(),
        key.position(),
    )
}

#[cfg(test)]
mod tests;
