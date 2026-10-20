//! Java Edition OptiFine/Iris shader-pack loading and configuration metadata.
//!
//! Open a pack with [`ShaderPack::open`], inspect [`ShaderPack::options`], select a
//! [`ShaderPack::dimension_folder`] and discover [`ShaderPack::programs`]. Chosen options
//! and an [`Environment`] supply the macros used by [`ShaderPack::properties`] and
//! [`ProgramRule::is_enabled`]. [`PackSource`] also exposes sorted text and bounded
//! binary asset reads from folders, ZIP archives or memory.
//!
//! Java world identities use the pinned Bedrock→Java mappings: [`ShaderPack::id_maps`]
//! reads a pack's block, item and entity ids and [`IdMaps`] resolves them for Bedrock
//! blocks, entities and items; [`BiomeUniforms`] and the `BIOME_*`/`CAT_*`/`PPT_*` macros
//! describe Java biomes; [`sky_fog`] gives Java's sky and fog colours.
//!
//! Shader translation, render-host integration and in-game acceptance remain incomplete.
//! Parsing custom uniform expressions does not execute or render them.
mod biomes;
mod dimensions;
mod environment;
mod expr;
mod ids;
mod java_sky;
mod options;
mod pack;
mod programs;
mod properties;
mod source;

pub use biomes::{BiomeCategory, BiomeUniforms};
pub use environment::{Environment, GpuVendor, HostOs};
pub use expr::{BinaryOp, Expr, ExprError, UnaryOp};
pub use ids::{BlockIdMap, IdMaps, NamedIdMap, RenderLayer, UNMAPPED_BLOCK_ID};
pub use java_sky::{BiomeColors, FogMedium, SkyFogColors, SkyFogInputs, sky_fog, sunrise_color};
pub use options::{OptionKind, OptionValues, PackOption};
pub use pack::ShaderPack;
pub use programs::{
    GEOMETRY_FALLBACKS, ProgramGroup, ProgramKind, ProgramSource, StageKind, geometry_fallback,
    program_group,
};
pub use properties::{
    BufferSize, CustomImage, CustomTexture, CustomUniform, CustomUniformKind, FlipRule, ImageSize,
    ProgramRule, PropertiesDiagnostic, ScaleRule, ShaderProperties, SizeValue, TextureSource,
};
pub use source::{PackError, PackOrigin, PackSource};
