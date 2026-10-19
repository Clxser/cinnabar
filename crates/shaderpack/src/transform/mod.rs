//! Compatibility-profile / GLSL 1.20 → Vulkan-flavoured GLSL 4.50 for naga's GLSL frontend.
//!
//! Works on the fully preprocessed token stream (no macros left), so a light parser over
//! top-level declarations is enough; function bodies are only rewritten at identifier level.
//! Each stage becomes a [`StageDraft`]; a program's drafts are assembled with one shared
//! loose-uniform block so every stage binds the same layout.

mod assemble;
mod body;
mod interface;
mod items;

use crate::token::Tok;
use assemble::UboMember;
use body::SamplerUse;

/// A transform failure at a 1-based line of the include-expanded source (0 if unknown).
#[derive(Debug)]
pub(crate) struct TransformError {
    pub message: String,
    pub line: u32,
}

impl TransformError {
    pub fn new(message: impl Into<String>, line: u32) -> TransformError {
        TransformError {
            message: message.into(),
            line,
        }
    }
}

/// One translated stage, waiting for the program's merged uniform block.
pub(crate) struct StageDraft {
    header: String,
    hoisted: Vec<Tok>,
    pub ubo: Vec<UboMember>,
    resources: Vec<Tok>,
    samplers: SamplerUse,
    builtin_io: String,
    io: Vec<Tok>,
    split_globals: Vec<Tok>,
    body: Vec<Tok>,
    /// Vertex inputs as (location, translated name).
    pub vertex_inputs: Vec<(u32, String)>,
    /// Fragment outputs as (location, translated name).
    pub frag_outputs: Vec<(u32, String)>,
    /// Pack-visible resource names as (group, binding, name).
    pub resource_names: Vec<(u32, u32, String)>,
}
