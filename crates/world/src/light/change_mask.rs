use super::{LightChannel, SubChunkLight};
use crate::SUB_CHUNK_SIDE;

const AXIS_NEIGHBOURS: u32 = 3;
const NEIGHBOURS: u32 = AXIS_NEIGHBOURS.pow(3);

/// Sections whose one-cell sampling halo can observe changed light in a source section.
/// Offset zero names the source; edges and corners require changed samples on that exact edge.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LightChangeMask(u32);

impl LightChangeMask {
    pub const EMPTY: Self = Self(0);
    pub const ALL: Self = Self((1 << NEIGHBOURS) - 1);
    pub const SOURCE: Self = Self(1 << Self::index([0; 3]));

    /// Whether the mask includes the relative section offset, rejecting offsets outside the halo.
    #[must_use]
    pub const fn contains(self, offset: [i32; 3]) -> bool {
        let [x, y, z] = offset;
        if x < -1 || x > 1 || y < -1 || y > 1 || z < -1 || z > 1 {
            return false;
        }
        self.0 & (1 << Self::index(offset)) != 0
    }

    /// Empty masks mean channel values did not change, even if their solve generation changed.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    const fn index([x, y, z]: [i32; 3]) -> u32 {
        ((x + 1) as u32 * AXIS_NEIGHBOURS + (y + 1) as u32) * AXIS_NEIGHBOURS + (z + 1) as u32
    }

    /// Includes exactly the halo regions that contain the changed local sample.
    fn include_sample(&mut self, [x, y, z]: [usize; 3]) {
        let range = |axis| match axis {
            0 => -1..=0,
            axis if axis == SUB_CHUNK_SIDE - 1 => 0..=1,
            _ => 0..=0,
        };
        for dx in range(x) {
            for dy in range(y) {
                for dz in range(z) {
                    self.0 |= 1 << Self::index([dx, dy, dz]);
                }
            }
        }
    }
}

impl SubChunkLight {
    /// Computes mesh invalidations on a worker, without treating unrelated changed faces as edges.
    /// A missing previous solve conservatively includes every neighbour.
    #[must_use]
    pub fn mesh_changes_since(&self, previous: Option<&Self>) -> LightChangeMask {
        let Some(previous) = previous else {
            return LightChangeMask::ALL;
        };
        if self.same_levels(previous) {
            return LightChangeMask::EMPTY;
        }
        let channels = [LightChannel::Block, LightChannel::Sky];
        if channels.iter().any(|&channel| {
            self.channel(channel).is_uniform()
                && previous.channel(channel).is_uniform()
                && self.channel(channel) != previous.channel(channel)
        }) {
            return LightChangeMask::ALL;
        }
        let mut changed = LightChangeMask::SOURCE;
        let edge = SUB_CHUNK_SIDE - 1;
        for x in 0..SUB_CHUNK_SIDE {
            for z in 0..SUB_CHUNK_SIDE {
                for y in 0..SUB_CHUNK_SIDE {
                    if x != 0 && x != edge && y != 0 && y != edge && z != 0 && z != edge {
                        continue;
                    }
                    if channels.iter().any(|&channel| {
                        self.get(channel, x as u8, y as u8, z as u8)
                            != previous.get(channel, x as u8, y as u8, z as u8)
                    }) {
                        changed.include_sample([x, y, z]);
                        if changed == LightChangeMask::ALL {
                            return changed;
                        }
                    }
                }
            }
        }
        changed
    }
}

#[cfg(test)]
mod tests;
