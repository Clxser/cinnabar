//! Neighbor-derived fence and pane states used by placement collision checks.

use assets::facts::connection::{ConnectionFacts, SIDES, connects_to};
use assets::{ContributorRole, ModelFamily, NetworkIdMode};
use serde_json::{Map, Value};
use sim::PaletteWorld;

use crate::{block_use::placement_cell, movement::PhysicsCollisionRegistries};

/// Resolves the same connections as rendering before selecting a registered collision shape.
pub(crate) fn states_for_neighbours(
    collisions: &PhysicsCollisionRegistries,
    mode: NetworkIdMode,
    world: &PaletteWorld<'_>,
    position: [i32; 3],
    identity: u32,
    states: &mut Map<String, Value>,
) -> Option<()> {
    let block = collisions.block_state_runtime_id(
        mode,
        assets::facts::BlockStateIdentity::Compiled(identity),
        states,
    )?;
    let center = collisions.block_connection_facts(mode, block)?;
    // Unclassified server blocks cannot silently count as disconnected neighbors.
    connected_states(states, center, |face| {
        let neighbor = world
            .primary_runtime_id(placement_cell(position, face))
            .ok()?;
        collisions.block_connection_facts(mode, neighbor)
    })
}

/// Sets all four links while retaining the palette's typed state envelopes.
fn connected_states(
    states: &mut Map<String, Value>,
    center: ConnectionFacts,
    mut neighbor: impl FnMut(u8) -> Option<ConnectionFacts>,
) -> Option<()> {
    if !matches!(center.family, ModelFamily::Fence | ModelFamily::Pane) {
        return Some(());
    }
    if center.role != ContributorRole::Primary {
        return None;
    }
    for side in SIDES {
        let connects = connects_to(center, neighbor(side.face)?, side.face)?;
        set_connection(states, side.state.name(), connects)?;
    }
    Some(())
}

/// Preserves byte and boolean palette representations while replacing one connection bit.
fn set_connection(states: &mut Map<String, Value>, key: &str, connected: bool) -> Option<()> {
    let entry = states.get_mut(key)?;
    let value = match entry {
        Value::Object(typed) => typed.get_mut("value")?,
        plain => plain,
    };
    *value = match value {
        Value::Bool(_) => Value::from(connected),
        Value::Number(number) if number.as_u64().is_some() => Value::from(u8::from(connected)),
        _ => return None,
    };
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Supplies primary model facts with no full-face occlusion.
    fn facts(family: ModelFamily) -> ConnectionFacts {
        ConnectionFacts {
            family,
            role: ContributorRole::Primary,
            occludes_full_face: false,
            links_as_unit_cube: false,
            orientation: None,
            nether_fence: false,
        }
    }

    /// Reads the pinned pane's typed connection entries.
    fn connection_states() -> Map<String, Value> {
        let records = assets::read_registry_for_protocol(
            assets::pinned_block_registry_bytes(),
            assets::active_content_registry_protocol(),
        )
        .unwrap();
        let pane = records
            .iter()
            .find(|record| record.name.as_ref() == "minecraft:glass_pane")
            .unwrap();
        serde_json::from_str(&pane.canonical_state).unwrap()
    }

    /// Panes link to every shipped unit cube, including the see-through ones the registry
    /// leaves without a cube model family.
    #[test]
    fn panes_link_to_shipped_see_through_cubes() {
        let protocol = assets::active_content_registry_protocol();
        let breg = assets::pinned_block_registry_bytes();
        let records = assets::read_registry_for_protocol(breg, protocol).unwrap();
        let collisions = PhysicsCollisionRegistries::from_assets(
            breg,
            &records,
            include_bytes!("../../assets/data/block-physics-v2225.bin"),
            protocol,
        )
        .unwrap();
        let facts_of = |name: &str| {
            let record = records
                .iter()
                .find(|record| record.name.as_ref() == name)
                .unwrap_or_else(|| panic!("pinned registry has {name}"));
            collisions
                .block_connection_facts(NetworkIdMode::Sequential, record.sequential_id)
                .unwrap_or_else(|| panic!("{name} has connection facts"))
        };
        let pane = facts_of("minecraft:glass_pane");
        for name in [
            "minecraft:glass",
            "minecraft:tinted_glass",
            "minecraft:ice",
            "minecraft:frosted_ice",
            "minecraft:honey_block",
            "minecraft:slime",
            "minecraft:oak_leaves",
        ] {
            assert_eq!(connects_to(pane, facts_of(name), 4), Some(true), "{name}");
        }
        for name in ["minecraft:poppy", "minecraft:torch"] {
            assert_eq!(connects_to(pane, facts_of(name), 4), Some(false), "{name}");
        }
    }

    #[test]
    fn links_are_written_before_collision_identity_lookup() {
        for (linked_face, linked_key) in [
            (2, "minecraft:connection_north"),
            (3, "minecraft:connection_south"),
            (4, "minecraft:connection_west"),
            (5, "minecraft:connection_east"),
        ] {
            let mut placed = connection_states();
            connected_states(&mut placed, facts(ModelFamily::Pane), |face| {
                Some(if face == linked_face {
                    facts(ModelFamily::Pane)
                } else {
                    facts(ModelFamily::Cross)
                })
            })
            .unwrap();
            for (key, entry) in &placed {
                assert_eq!(entry["value"], u8::from(key.as_str() == linked_key));
            }
            assert!(connected_states(&mut placed, facts(ModelFamily::Pane), |_| None).is_none());
        }
    }
}
