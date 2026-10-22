//! Frozen prepared terrain geometry, retaining greedy quads, model templates, liquids and biome halos.

use std::{
    io::{Read, Write},
    sync::Arc,
};

use assets::RuntimeAssets;
use meshing::{
    CubeQuadLayout, Face, PackedBiomeRecord, PackedLiquidQuad, PackedModelDrawRef, PackedModelRef,
    PackedQuad, PackedQuadLighting,
};
use recording_studio::archive::{ArchiveError, MAX_RECORD_BYTES};

use crate::binary::{self, Reader};

const MAGIC: [u8; 8] = *b"CINTER01";

#[derive(Clone, Debug)]
pub struct TerrainMesh {
    pub cubes: Arc<[PackedQuad]>,
    pub cube_lighting: Arc<[PackedQuadLighting]>,
    pub cube_layout: CubeQuadLayout,
    pub models: Arc<[PackedModelRef]>,
    pub model_lighting: Arc<[PackedQuadLighting]>,
    pub model_draws: Arc<[PackedModelDrawRef]>,
    pub transparent_model_draws: Arc<[PackedModelDrawRef]>,
    pub liquids: Arc<[PackedLiquidQuad]>,
    pub liquid_lighting: Arc<[PackedQuadLighting]>,
    pub biome: PackedBiomeRecord,
}

impl TerrainMesh {
    /// Counts every pinned stream, including the exact palette-native biome record.
    pub fn retained_bytes(&self) -> usize {
        std::mem::size_of_val(&*self.cubes)
            + std::mem::size_of_val(&*self.cube_lighting)
            + std::mem::size_of_val(&*self.models)
            + std::mem::size_of_val(&*self.model_lighting)
            + std::mem::size_of_val(&*self.model_draws)
            + std::mem::size_of_val(&*self.transparent_model_draws)
            + std::mem::size_of_val(&*self.liquids)
            + std::mem::size_of_val(&*self.liquid_lighting)
            + self.biome.words().len() * std::mem::size_of::<u32>()
    }

    /// Cross-references use the frozen material/model tables, sharing live GPU model admission.
    pub fn validate(&self, assets: &RuntimeAssets) -> Result<(), ArchiveError> {
        if self.retained_bytes() > MAX_RECORD_BYTES.saturating_sub(256)
            || self.cubes.len() != self.cube_lighting.len()
            || self.cube_layout.checked(&self.cubes, assets.materials()) != self.cube_layout
            || self.cubes.iter().any(|quad| {
                quad.material_id() != assets::DIAGNOSTIC_MATERIAL
                    && quad.material_id() as usize >= assets.material_count()
            })
            || self.liquids.iter().any(|quad| {
                quad.words()[3] as usize >= self.liquid_lighting.len()
                    || quad.material_id() as usize >= assets.material_count()
            })
            || !meshing::model_validation::validate_partitioned_model_streams(
                &self.models,
                &self.model_lighting,
                &self.model_draws,
                &self.transparent_model_draws,
                assets.model_templates(),
                assets.model_quads(),
                assets.materials(),
            )
        {
            return Err(ArchiveError::Invalid(
                "recorded terrain streams do not match their frozen asset tables",
            ));
        }
        let depth = self
            .liquids
            .iter()
            .position(|quad| quad.is_depth_writing())
            .unwrap_or(self.liquids.len());
        if self.liquids[depth..]
            .iter()
            .any(|quad| !quad.is_depth_writing())
        {
            return Err(ArchiveError::Invalid(
                "recorded liquid depth partition is invalid",
            ));
        }
        Ok(())
    }

    /// Streams explicit words into a content object without cloning immutable mesh arrays.
    pub fn write(&self, output: &mut dyn Write) -> Result<(), ArchiveError> {
        if self.retained_bytes() > MAX_RECORD_BYTES.saturating_sub(256) {
            return Err(ArchiveError::Invalid(
                "recorded terrain object is too large",
            ));
        }
        output
            .write_all(&MAGIC)
            .map_err(|_| ArchiveError::Invalid("cannot write terrain object header"))?;
        for face in Face::ALL {
            let range = self.cube_layout.solid_range(face);
            binary::u32(output, range.end - range.start)?;
        }
        binary::packed(output, &self.cubes, PackedQuad::words)?;
        write_lights(output, &self.cube_lighting)?;
        binary::packed(output, &self.models, |model| model.words())?;
        write_lights(output, &self.model_lighting)?;
        binary::packed(output, &self.model_draws, |draw| draw.words())?;
        binary::packed(output, &self.transparent_model_draws, |draw| draw.words())?;
        binary::packed(output, &self.liquids, |quad| quad.words())?;
        write_lights(output, &self.liquid_lighting)?;
        binary::packed(output, self.biome.words(), |&word| [word])
    }

    /// A seek worker reconstructs typed immutable arrays only after bounded field admission.
    pub fn read(
        input: &mut dyn Read,
        assets: &RuntimeAssets,
        tint_count: usize,
    ) -> Result<Self, ArchiveError> {
        let mut reader = Reader::new(input);
        if reader.bytes::<8>()? != MAGIC {
            return Err(ArchiveError::Invalid("unsupported recorded terrain schema"));
        }
        let mut counts = [0_u32; Face::ALL.len()];
        let mut total = 0_u32;
        for count in &mut counts {
            *count = reader.u32()?;
            total = total
                .checked_add(*count)
                .ok_or(ArchiveError::Invalid("recorded solid ranges overflow"))?;
        }
        let cubes = reader.packed(PackedQuad::try_from_words)?;
        if total as usize > cubes.len() {
            return Err(ArchiveError::Invalid(
                "recorded solid ranges exceed their quad stream",
            ));
        }
        let cube_lighting = read_lights(&mut reader)?;
        let models = reader.packed(|words: [u32; 4]| {
            Some(PackedModelRef::new(words[0], words[1], words[2], words[3]))
        })?;
        let model_lighting = read_lights(&mut reader)?;
        let model_draws =
            reader.packed(|words: [u32; 2]| Some(PackedModelDrawRef::new(words[0], words[1])))?;
        let transparent_model_draws =
            reader.packed(|words: [u32; 2]| Some(PackedModelDrawRef::new(words[0], words[1])))?;
        let liquids = reader.packed(PackedLiquidQuad::try_from_words)?;
        let liquid_lighting = read_lights(&mut reader)?;
        let biome_words = reader.packed(|words: [u32; 1]| Some(words[0]))?;
        drop(reader);
        binary::finish(input)?;
        let biome = PackedBiomeRecord::from_recorded_words(biome_words.into(), tint_count)
            .ok_or(ArchiveError::Invalid("invalid recorded biome halo"))?;
        let mesh = Self {
            cubes: cubes.into(),
            cube_lighting: cube_lighting.into(),
            cube_layout: CubeQuadLayout::from_solid_counts(counts),
            models: models.into(),
            model_lighting: model_lighting.into(),
            model_draws: model_draws.into(),
            transparent_model_draws: transparent_model_draws.into(),
            liquids: liquids.into(),
            liquid_lighting: liquid_lighting.into(),
            biome,
        };
        mesh.validate(assets)?;
        Ok(mesh)
    }
}

/// Two little-endian words preserve four independent packed light/AO samples exactly.
fn write_lights(output: &mut dyn Write, lights: &[PackedQuadLighting]) -> Result<(), ArchiveError> {
    binary::packed(output, lights, |light| {
        let [a, b, c, d] = light.samples();
        [
            u32::from(a) | (u32::from(b) << u16::BITS),
            u32::from(c) | (u32::from(d) << u16::BITS),
        ]
    })
}

/// Reverses the packed sidecar representation without assuming host endian or struct padding.
fn read_lights(reader: &mut Reader<'_>) -> Result<Vec<PackedQuadLighting>, ArchiveError> {
    reader.packed(|words: [u32; 2]| {
        Some(PackedQuadLighting::new([
            words[0] as u16,
            (words[0] >> u16::BITS) as u16,
            words[1] as u16,
            (words[1] >> u16::BITS) as u16,
        ]))
    })
}
