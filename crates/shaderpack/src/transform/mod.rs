//! Compatibility-profile / GLSL 1.20 → Vulkan-flavoured GLSL 4.50 for naga's GLSL frontend.
//!
//! Works on the fully preprocessed token stream (no macros left), so a light parser over
//! top-level declarations is enough; function bodies are only rewritten at identifier level.

mod interface;
mod items;
