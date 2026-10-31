//! Pane and fence links that the neighbouring block grants on its own.
//!
//! Native connections accept any neighbour without a connection rule. Unit cubes carry
//! none, so panes and fences join them whatever their own family. Rendering and placement
//! collision both read this one classification so their shapes cannot diverge.

use std::sync::OnceLock;

use crate::block_shapes::ShapeBox;
use crate::registry::BlockTraits;
use crate::{
    BlockFlags, ContributorRole, ModelFamily, ModelStateField, NetworkIdMode, RegistryRecord,
    facts::{BlockProperty, block_render_fact},
};

/// One horizontal side a pane or fence links across.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Side {
    /// Offset of the neighbour across this side.
    pub offset: [i32; 3],
    /// Bedrock face index pointing at the neighbour.
    pub face: u8,
    /// Palette state that stores this side's link.
    pub state: BlockProperty,
}

/// The four linkable sides in the west, east, north, south order of link arrays.
pub const SIDES: [Side; 4] = [
    Side {
        offset: [-1, 0, 0],
        face: 4,
        state: BlockProperty::MinecraftConnectionWest,
    },
    Side {
        offset: [1, 0, 0],
        face: 5,
        state: BlockProperty::MinecraftConnectionEast,
    },
    Side {
        offset: [0, 0, -1],
        face: 2,
        state: BlockProperty::MinecraftConnectionNorth,
    },
    Side {
        offset: [0, 0, 1],
        face: 3,
        state: BlockProperty::MinecraftConnectionSouth,
    },
];

/// Registry facts that pane, fence and gate connection predicates read from a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectionFacts {
    pub family: ModelFamily,
    pub role: ContributorRole,
    pub occludes_full_face: bool,
    /// Panes and fences link to it on every side; see [`links_as_unit_cube`].
    pub links_as_unit_cube: bool,
    /// The gate axis selector, when the state has one.
    pub orientation: Option<u32>,
    pub nether_fence: bool,
}

impl ConnectionFacts {
    /// Reads the connection facts of a built-in block state.
    #[must_use]
    pub fn from_record(record: &RegistryRecord) -> Self {
        Self {
            family: record.model_family,
            role: record.contributor_role,
            occludes_full_face: record.flags.contains(BlockFlags::OCCLUDES_FULL_FACE),
            links_as_unit_cube: links_as_unit_cube(record),
            orientation: record.model_state.get(ModelStateField::Orientation),
            nether_fence: record.traits.contains(BlockTraits::NETHER_BRICK_FENCE),
        }
    }

    /// Whether this block derives its links from its neighbours rather than its palette state.
    #[must_use]
    pub fn links_from_neighbours(self) -> bool {
        self.role == ContributorRole::Primary
            && matches!(self.family, ModelFamily::Pane | ModelFamily::Fence)
    }
}

/// Whether `center` links to `neighbor` across Bedrock `face`: panes to panes, walls, full-face
/// occluders and unit cubes; fences to same-material fences, perpendicular gates, full-face
/// occluders and unit cubes. `None` when a fact the rule needs is unknown.
#[must_use]
pub fn connects_to(center: ConnectionFacts, neighbor: ConnectionFacts, face: u8) -> Option<bool> {
    if neighbor.occludes_full_face {
        return Some(true);
    }
    if neighbor.role != ContributorRole::Primary {
        return Some(false);
    }
    if neighbor.links_as_unit_cube {
        return Some(true);
    }
    match center.family {
        ModelFamily::Pane => Some(matches!(
            neighbor.family,
            ModelFamily::Pane | ModelFamily::Wall
        )),
        ModelFamily::Fence => match neighbor.family {
            ModelFamily::Fence => Some(center.nether_fence == neighbor.nether_fence),
            ModelFamily::Gate => {
                let orientation = neighbor.orientation?;
                if orientation > 3 {
                    return None;
                }
                Some(match face {
                    2 | 3 => orientation & 1 != 0,
                    4 | 5 => orientation & 1 == 0,
                    _ => return None,
                })
            }
            _ => Some(false),
        },
        _ => None,
    }
}

/// The single box a pane or fence picks and outlines: its post widened to each linked side
/// (`links` in [`SIDES`] order), one block tall. `None` for other families.
#[must_use]
pub fn visual_box(family: ModelFamily, links: [bool; 4]) -> Option<ShapeBox> {
    let inset = match family {
        ModelFamily::Pane => 0.4375,
        ModelFamily::Fence => 0.375,
        _ => return None,
    };
    let [west, east, north, south] = links;
    Some(ShapeBox::new(
        [
            if west { 0.0 } else { inset },
            0.0,
            if north { 0.0 } else { inset },
        ],
        [
            if east { 1.0 } else { 1.0 - inset },
            1.0,
            if south { 1.0 } else { 1.0 - inset },
        ],
    ))
}

/// Whether panes and fences link to `record` as a unit cube: an opaque or leaf cube, glass,
/// or one of the see-through cubes the registry leaves without a cube model family
/// (ice, slime, honey, tinted glass, powder snow).
#[must_use]
pub fn links_as_unit_cube(record: &RegistryRecord) -> bool {
    record.contributor_role == ContributorRole::Primary
        && !record.flags.contains(BlockFlags::AIR)
        && (record
            .flags
            .intersects(BlockFlags::CUBE_GEOMETRY | BlockFlags::LEAF_MODEL)
            || block_render_fact(&record.name).kind.is_translucent_cube())
}

/// [`links_as_unit_cube`] for a built-in block state. Custom and unknown states are `false`.
#[must_use]
pub fn pinned_links_as_unit_cube(mode: NetworkIdMode, runtime_id: u32) -> bool {
    static LINKS: OnceLock<Box<[bool]>> = OnceLock::new();
    let links = LINKS.get_or_init(|| {
        let records = crate::read_registry_for_protocol(
            crate::pinned_block_registry_bytes(),
            crate::active_content_registry_protocol(),
        )
        .expect("validated pinned block registry");
        let mut links = vec![false; records.len()];
        for record in &records {
            if let Some(link) = links.get_mut(record.sequential_id as usize) {
                *link = links_as_unit_cube(record);
            }
        }
        links.into_boxed_slice()
    });
    let id = match mode {
        NetworkIdMode::Sequential => Some(runtime_id),
        NetworkIdMode::Hashed => crate::pinned_block_sequential_id(runtime_id),
    };
    id.and_then(|id| links.get(id as usize).copied())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pinned(name: &str) -> RegistryRecord {
        crate::read_registry_for_protocol(
            crate::pinned_block_registry_bytes(),
            crate::active_content_registry_protocol(),
        )
        .unwrap()
        .into_iter()
        .find(|record| record.name.as_ref() == name)
        .unwrap_or_else(|| panic!("pinned registry has {name}"))
    }

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

    #[test]
    fn panes_connect_to_panes_walls_and_full_face_occluders() {
        let pane = facts(ModelFamily::Pane);
        for family in [ModelFamily::Pane, ModelFamily::Wall] {
            assert_eq!(connects_to(pane, facts(family), 4), Some(true));
        }
        assert_eq!(connects_to(pane, facts(ModelFamily::Fence), 4), Some(false));
        let mut opaque = facts(ModelFamily::Cross);
        opaque.occludes_full_face = true;
        assert_eq!(connects_to(pane, opaque, 4), Some(true));
        assert_eq!(connects_to(pane, facts(ModelFamily::Cross), 4), Some(false));
    }

    #[test]
    fn panes_and_fences_connect_to_see_through_unit_cubes() {
        for center in [facts(ModelFamily::Pane), facts(ModelFamily::Fence)] {
            for family in [ModelFamily::Cube, ModelFamily::Leaves, ModelFamily::Unknown] {
                let mut cube = facts(family);
                cube.links_as_unit_cube = true;
                assert_eq!(connects_to(center, cube, 4), Some(true));
            }
        }
    }

    #[test]
    fn wood_and_nether_fences_connect_only_to_the_same_group() {
        let wood = facts(ModelFamily::Fence);
        let mut nether = wood;
        nether.nether_fence = true;
        for (center, neighbor, expected) in [
            (wood, wood, true),
            (nether, nether, true),
            (wood, nether, false),
            (nether, wood, false),
        ] {
            assert_eq!(connects_to(center, neighbor, 2), Some(expected));
        }
        assert!(ConnectionFacts::from_record(&pinned("minecraft:nether_brick_fence")).nether_fence);
        assert!(!ConnectionFacts::from_record(&pinned("minecraft:spruce_fence")).nether_fence);
    }

    #[test]
    fn fence_gate_connection_uses_perpendicular_axis() {
        let fence = facts(ModelFamily::Fence);
        for orientation in 0..=3 {
            let mut gate = facts(ModelFamily::Gate);
            gate.orientation = Some(orientation);
            for face in 2..=5 {
                assert_eq!(
                    connects_to(fence, gate, face),
                    Some((orientation & 1 == 0) == (face >= 4))
                );
            }
        }
        assert_eq!(connects_to(fence, facts(ModelFamily::Gate), 2), None);
    }

    #[test]
    fn visual_box_widens_the_post_to_each_linked_side() {
        for (family, inset) in [(ModelFamily::Pane, 0.4375), (ModelFamily::Fence, 0.375)] {
            assert_eq!(
                visual_box(family, [false; 4]),
                Some(ShapeBox::new(
                    [inset, 0.0, inset],
                    [1.0 - inset, 1.0, 1.0 - inset]
                ))
            );
            assert_eq!(
                visual_box(family, [false, true, true, false]),
                Some(ShapeBox::new([inset, 0.0, 0.0], [1.0, 1.0, 1.0 - inset]))
            );
            assert_eq!(
                visual_box(family, [true; 4]),
                Some(ShapeBox::new([0.0; 3], [1.0; 3]))
            );
        }
        assert_eq!(visual_box(ModelFamily::Wall, [true; 4]), None);
    }

    #[test]
    fn shipped_cubes_link_and_other_shapes_do_not() {
        for (name, links) in [
            ("minecraft:glass", true),
            ("minecraft:tinted_glass", true),
            ("minecraft:ice", true),
            ("minecraft:frosted_ice", true),
            ("minecraft:honey_block", true),
            ("minecraft:slime", true),
            ("minecraft:oak_leaves", true),
            ("minecraft:stone", true),
            ("minecraft:air", false),
            ("minecraft:poppy", false),
            ("minecraft:torch", false),
            ("minecraft:glass_pane", false),
            ("minecraft:oak_stairs", false),
        ] {
            let record = pinned(name);
            assert_eq!(links_as_unit_cube(&record), links, "{name}");
            for (mode, id) in [
                (NetworkIdMode::Sequential, record.sequential_id),
                (NetworkIdMode::Hashed, record.network_hash),
            ] {
                assert_eq!(
                    pinned_links_as_unit_cube(mode, id),
                    links,
                    "{name} {mode:?}"
                );
            }
        }
    }
}
