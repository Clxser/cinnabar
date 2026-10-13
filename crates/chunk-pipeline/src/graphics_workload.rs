//! A bounded single-core light and mesh workload using the Criterion terrain fixtures.

use std::hint::black_box;

use meshing::{
    BlockClassifier, MeshLightSample, mesh_sub_chunk_in_neighbourhood_with_lighting,
    synthetic_terrain,
};
use world::{
    BlockPos, DimensionLightProfile, EmptyLight, LightBlockAccess, LightBlockSample, LightBounds,
    LightProperties, LightSolverScratch, SolverLimits, solve_light_full_with_scratch,
};
use world::{LightChannel, MeshNeighbourhood, SubChunk};

/// Holds two fixed sections and worker-local scratch; never starts streaming workers.
pub struct GraphicsWorkload {
    assets: assets::RuntimeAssets,
    sections: [SubChunk; 2],
    empty: SubChunk,
    scratch: LightSolverScratch,
    next: usize,
}

impl Default for GraphicsWorkload {
    fn default() -> Self {
        Self {
            assets: synthetic_terrain::terrain_assets(),
            sections: [
                synthetic_terrain::mixed_chunk(),
                synthetic_terrain::chunk_of(synthetic_terrain::sparse_id),
            ],
            empty: synthetic_terrain::uniform_chunk(synthetic_terrain::AIR),
            scratch: LightSolverScratch::default(),
            next: 0,
        }
    }
}

struct SectionBlocks<'a>(&'a SubChunk);

impl LightBlockAccess for SectionBlocks<'_> {
    fn sample(&self, position: BlockPos) -> LightBlockSample {
        if [position.x, position.y, position.z]
            .iter()
            .any(|v| !(0..16).contains(v))
        {
            return LightBlockSample::Unknown;
        }
        match self
            .0
            .runtime_id(0, position.x as u8, position.y as u8, position.z as u8)
        {
            Some(synthetic_terrain::AIR) => LightBlockSample::KnownAir,
            Some(id) => LightBlockSample::Resident(
                LightProperties::new(if id == 2 { 10 } else { 0 }, 15).expect("fixture light"),
            ),
            None => LightBlockSample::Unknown,
        }
    }

    fn sky_seed(&self, position: BlockPos) -> u8 {
        if position.y == 15 && self.sample(position) == LightBlockSample::KnownAir {
            15
        } else {
            0
        }
    }
}

impl GraphicsWorkload {
    /// Solves and meshes one 16-cubed section, allowing cancellation between sections.
    pub fn run_section(&mut self) {
        let section = &self.sections[self.next];
        self.next = (self.next + 1) % self.sections.len();
        let lights = solve_light_full_with_scratch(
            &SectionBlocks(section),
            &EmptyLight,
            LightBounds::new(0, BlockPos::new(0, 0, 0), BlockPos::new(15, 15, 15))
                .expect("fixture bounds"),
            1,
            DimensionLightProfile::Overworld {
                direct_sky_down: true,
            },
            SolverLimits::new(
                world::BLOCKS_PER_SUB_CHUNK,
                world::BLOCKS_PER_SUB_CHUNK * 32,
            ),
            &mut self.scratch,
        )
        .expect("bounded fixture light solve");
        let mut neighbourhood = MeshNeighbourhood::new(section);
        for offset in [
            [-1, 0, 0],
            [1, 0, 0],
            [0, -1, 0],
            [0, 1, 0],
            [0, 0, -1],
            [0, 0, 1],
        ] {
            neighbourhood.insert(offset, &self.empty);
        }
        let sampler = |position: [i32; 3]| {
            let position = BlockPos::from(position);
            MeshLightSample::try_new(
                lights.light_at(position, LightChannel::Block),
                lights.light_at(position, LightChannel::Sky),
            )
            .expect("solver light levels")
        };
        black_box(mesh_sub_chunk_in_neighbourhood_with_lighting(
            &BlockClassifier::new(synthetic_terrain::AIR),
            &self.assets,
            assets::NetworkIdMode::Sequential,
            &neighbourhood,
            &sampler,
        ));
    }
}
