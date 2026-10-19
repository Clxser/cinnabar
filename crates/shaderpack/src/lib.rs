//! Java Edition OptiFine/Iris shader-pack loading and configuration metadata.
//!
//! Open a pack with [`ShaderPack::open`], inspect [`ShaderPack::options`], select a
//! [`ShaderPack::dimension_folder`] and discover [`ShaderPack::programs`]. Chosen options
//! and an [`Environment`] supply the macros used by [`ShaderPack::properties`] and
//! [`ProgramRule::is_enabled`]. [`PackSource`] also exposes sorted text and bounded
//! binary asset reads from folders, ZIP archives or memory.
//!
//! Shader translation, render-host integration and in-game acceptance remain incomplete.
#[allow(
    dead_code,
    reason = "wired into ShaderPack::translate later in the translator stack"
)]
mod diagnostics;
mod dimensions;
mod environment;
mod expr;
mod options;
mod pack;
mod programs;
mod properties;
mod source;
#[allow(
    dead_code,
    reason = "wired into ShaderPack::translate later in the translator stack"
)]
mod token;

pub use diagnostics::{Diagnostic, Phase, SourceLocation};
pub use environment::{Environment, GpuVendor, HostOs};
pub use expr::{BinaryOp, Expr, ExprError, UnaryOp};
pub use options::{OptionKind, OptionValues, PackOption};
pub use pack::ShaderPack;
pub use programs::{
    GEOMETRY_FALLBACKS, ProgramGroup, ProgramKind, ProgramSource, StageKind, geometry_fallback,
    program_group,
};
pub use properties::{CustomImage, ImageSize, ProgramRule, PropertiesDiagnostic, ShaderProperties};
pub use source::{PackError, PackOrigin, PackSource};
