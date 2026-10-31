//! Panes and fences pick and outline the arms their neighbours link, not their palette links.

use super::{Aabb, BlockPhysics, CollisionRegistry, PaletteWorld, WorldQueryError, box_to_aabb};
use assets::facts::connection::{self, ConnectionFacts};

impl CollisionRegistry {
    /// Attaches the facts that neighbour-derived pane and fence links read from this block.
    pub fn set_connection_facts(&mut self, id: u32, facts: ConnectionFacts) -> bool {
        let Some(block) = std::sync::Arc::make_mut(&mut self.blocks).get_mut(id) else {
            return false;
        };
        block.connection = Some(facts);
        true
    }
}

impl PaletteWorld<'_> {
    /// Resolves a pane or fence's box from the neighbours it links to. `None` keeps the
    /// compiled state boxes: other blocks, and neighbours whose link facts are unknown.
    pub(super) fn connected_selection(
        &self,
        position: [i32; 3],
        physics: &BlockPhysics,
        lookup: &mut impl FnMut([i32; 3]) -> Result<u32, WorldQueryError>,
    ) -> Result<Option<Aabb>, WorldQueryError> {
        let Some(center) = physics
            .connection
            .filter(|facts| facts.links_from_neighbours())
        else {
            return Ok(None);
        };
        let mut links = [false; 4];
        for (link, side) in links.iter_mut().zip(connection::SIDES) {
            let mut neighbour = position;
            for axis in 0..3 {
                neighbour[axis] = neighbour[axis]
                    .checked_add(side.offset[axis])
                    .ok_or(WorldQueryError::CoordinateOutOfRange)?;
            }
            let id = lookup(neighbour)?;
            let Some(connects) = self
                .registry
                .physics(id)
                .and_then(|other| other.connection)
                .and_then(|other| connection::connects_to(center, other, side.face))
            else {
                return Ok(None);
            };
            *link = connects;
        }
        Ok(connection::visual_box(center.family, links).map(box_to_aabb))
    }
}
