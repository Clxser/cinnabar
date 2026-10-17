//! The expanded Iris vertex: its word layout, the vertex buffer layouts a translated program's
//! inputs bind to, and the constant stream for attributes terrain does not carry.

use bevy::mesh::VertexBufferLayout;
use bevy::render::render_resource::{
    VertexAttribute as GpuVertexAttribute, VertexFormat, VertexStepMode,
};
use shaderpack::{ScalarType, ValueType, VertexAttribute, VertexInput};

/// One stored vertex field: its first word in the expanded vertex and the format it is
/// written in. The expansion kernels receive these words as generated WGSL constants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StoredField {
    pub(crate) placeholder: &'static str,
    pub(crate) word: u32,
    pub(crate) format: VertexFormat,
}

const fn field(placeholder: &'static str, word: u32, format: VertexFormat) -> StoredField {
    StoredField {
        placeholder,
        word,
        format,
    }
}

/// Position relative to the geometry origin (`gl_Vertex` / `vaPosition`).
pub(crate) const POSITION: StoredField = field("IRIS_WORD_POSITION", 0, VertexFormat::Float32x3);
/// Gamma tint times AO, or AO in alpha under `separateAo` (`gl_Color`).
pub(crate) const COLOR: StoredField = field("IRIS_WORD_COLOR", 3, VertexFormat::Unorm8x4);
/// Atlas texture coordinates (`gl_MultiTexCoord0` / `vaUV0`).
pub(crate) const UV0: StoredField = field("IRIS_WORD_UV0", 4, VertexFormat::Unorm16x2);
/// Mean atlas coordinates of the face (`mc_midTexCoord`).
pub(crate) const MID_TEX: StoredField = field("IRIS_WORD_MID_TEX", 5, VertexFormat::Unorm16x2);
/// Java lightmap coordinates, light level × 16 for block then sky (`gl_MultiTexCoord1/2`).
pub(crate) const LIGHTMAP_FLOAT: StoredField =
    field("IRIS_WORD_LIGHTMAP_FLOAT", 6, VertexFormat::Float16x2);
/// The same coordinates rounded, for the core profile's `ivec2 vaUV2`.
pub(crate) const LIGHTMAP_INT: StoredField =
    field("IRIS_WORD_LIGHTMAP_INT", 7, VertexFormat::Sint16x2);
/// Face normal (`gl_Normal` / `vaNormal`).
pub(crate) const NORMAL: StoredField = field("IRIS_WORD_NORMAL", 8, VertexFormat::Snorm8x4);
/// Tangent with handedness in `w` (`at_tangent`).
pub(crate) const TANGENT: StoredField = field("IRIS_WORD_TANGENT", 9, VertexFormat::Snorm8x4);
/// `block.properties` id, -1 when unmapped (`mc_Entity.x`; `.y` reads 0).
pub(crate) const BLOCK_ID: StoredField = field("IRIS_WORD_BLOCK_ID", 10, VertexFormat::Float32);
/// Offset to the block centre in 1/64 blocks, and the block's emission (`at_midBlock`).
pub(crate) const MID_BLOCK: StoredField = field("IRIS_WORD_MID_BLOCK", 11, VertexFormat::Float16x4);

/// Every stored field, in word order.
pub(crate) const STORED_FIELDS: [StoredField; 10] = [
    POSITION,
    COLOR,
    UV0,
    MID_TEX,
    LIGHTMAP_FLOAT,
    LIGHTMAP_INT,
    NORMAL,
    TANGENT,
    BLOCK_ID,
    MID_BLOCK,
];

/// Words of one expanded vertex.
pub const IRIS_VERTEX_WORDS: u32 = {
    let last = STORED_FIELDS[STORED_FIELDS.len() - 1];
    last.word + (last.format.size() / 4) as u32
};
/// Bytes between consecutive expanded vertices.
pub const IRIS_VERTEX_STRIDE: u64 = IRIS_VERTEX_WORDS as u64 * 4;

const _: () = {
    let mut index = 1;
    while index < STORED_FIELDS.len() {
        let previous = STORED_FIELDS[index - 1];
        assert!(previous.word + (previous.format.size() / 4) as u32 == STORED_FIELDS[index].word);
        index += 1;
    }
};

/// Vertex buffer slot of the expanded geometry stream.
pub const GEOMETRY_VERTEX_BUFFER: u32 = 0;
/// Vertex buffer slot of the constant stream: a zero-stride buffer that supplies Iris's
/// fixed values for attributes terrain does not carry.
pub const CONSTANT_VERTEX_BUFFER: u32 = 1;

/// Words of the constant stream: zero floats, zero integers, chunk fade, overlay coordinates.
pub(crate) const CONSTANT_STREAM_WORDS: [u32; 13] = [
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    1.0_f32.to_bits(),
    0,
    NO_OVERLAY[1] as u32,
    0,
    (NO_OVERLAY[1] as f32).to_bits(),
];
const ZERO_FLOAT_WORD: u32 = 0;
const ZERO_INT_WORD: u32 = 4;
const CHUNK_FADE_WORD: u32 = 8;
const OVERLAY_INT_WORD: u32 = 9;
const OVERLAY_FLOAT_WORD: u32 = 11;
/// Java's `OverlayTexture.NO_OVERLAY` coordinates (`vaUV1`): no hurt tint, no flash.
const NO_OVERLAY: [i32; 2] = [0, 10];

/// Why a translated program's vertex input cannot bind to terrain geometry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VertexLayoutError {
    /// The input is declared with a type terrain vertices are not stored in.
    UnsupportedType { name: String, ty: ValueType },
}

impl std::fmt::Display for VertexLayoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedType { name, ty } => {
                write!(f, "vertex input `{name}` has unsupported type {ty:?}")
            }
        }
    }
}

impl std::error::Error for VertexLayoutError {}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Float,
    Sint,
    Uint,
}

fn class_and_size(ty: ValueType) -> Option<(Class, u8)> {
    let (scalar, size) = match ty {
        ValueType::Scalar(scalar) => (scalar, 1),
        ValueType::Vector { size, scalar } => (scalar, size),
        ValueType::Matrix { .. } => return None,
    };
    let class = match scalar {
        ScalarType::Float => Class::Float,
        ScalarType::Int => Class::Sint,
        ScalarType::Uint => Class::Uint,
        ScalarType::Bool | ScalarType::Double => return None,
    };
    Some((class, size))
}

/// The stored field and format an input reads from the expanded stream, or `None` when the
/// constant stream supplies it.
fn stored_format(attribute: &VertexAttribute, class: Class) -> Option<Option<(u32, VertexFormat)>> {
    let stored = |field: StoredField| Some(Some((field.word, field.format)));
    match (attribute, class) {
        (VertexAttribute::Position, Class::Float) => stored(POSITION),
        (VertexAttribute::Color, Class::Float) => stored(COLOR),
        (VertexAttribute::TexCoord, Class::Float) => stored(UV0),
        (VertexAttribute::MidTexCoord, Class::Float) => stored(MID_TEX),
        // Iris documents gl_MultiTexCoord2 as a second name for the lightmap coordinates.
        (VertexAttribute::Lightmap | VertexAttribute::MultiTexCoord2, Class::Float) => {
            stored(LIGHTMAP_FLOAT)
        }
        (VertexAttribute::Lightmap | VertexAttribute::MultiTexCoord2, Class::Sint) => {
            stored(LIGHTMAP_INT)
        }
        (VertexAttribute::Lightmap | VertexAttribute::MultiTexCoord2, Class::Uint) => {
            Some(Some((LIGHTMAP_INT.word, VertexFormat::Uint16x2)))
        }
        (VertexAttribute::Normal, Class::Float) => stored(NORMAL),
        (VertexAttribute::Tangent, Class::Float) => stored(TANGENT),
        (VertexAttribute::BlockId, Class::Float) => stored(BLOCK_ID),
        (VertexAttribute::MidBlock, Class::Float) => stored(MID_BLOCK),
        (
            VertexAttribute::MultiTexCoord3
            | VertexAttribute::Overlay
            | VertexAttribute::Velocity
            | VertexAttribute::ChunkFade
            | VertexAttribute::DhMaterialId
            | VertexAttribute::Other(_),
            _,
        ) => Some(None),
        _ => None,
    }
}

/// Format and word of a constant-stream input with `size` components of `class`.
fn constant_format(attribute: &VertexAttribute, class: Class, size: u8) -> (u32, VertexFormat) {
    use VertexFormat as F;
    let word = match (attribute, class) {
        (VertexAttribute::ChunkFade, Class::Float) => CHUNK_FADE_WORD,
        (VertexAttribute::Overlay, Class::Float) => OVERLAY_FLOAT_WORD,
        (VertexAttribute::Overlay, _) => OVERLAY_INT_WORD,
        (_, Class::Float) => ZERO_FLOAT_WORD,
        _ => ZERO_INT_WORD,
    };
    // Overlay and chunk fade carry their own component counts; zeros take the declared one.
    let size = match attribute {
        VertexAttribute::ChunkFade => 1,
        VertexAttribute::Overlay => size.min(2),
        _ => size,
    };
    let format = match (class, size) {
        (Class::Float, 1) => F::Float32,
        (Class::Float, 2) => F::Float32x2,
        (Class::Float, 3) => F::Float32x3,
        (Class::Float, _) => F::Float32x4,
        (Class::Sint, 1) => F::Sint32,
        (Class::Sint, 2) => F::Sint32x2,
        (Class::Sint, 3) => F::Sint32x3,
        (Class::Sint, _) => F::Sint32x4,
        (Class::Uint, 1) => F::Uint32,
        (Class::Uint, 2) => F::Uint32x2,
        (Class::Uint, 3) => F::Uint32x3,
        (Class::Uint, _) => F::Uint32x4,
    };
    (word, format)
}

/// Vertex buffer layouts binding a translated program's `inputs` (from
/// `ProgramReflection::vertex_inputs`) to terrain geometry: slot [`GEOMETRY_VERTEX_BUFFER`]
/// is the expanded stream and slot [`CONSTANT_VERTEX_BUFFER`] the zero-stride constant stream.
/// Fewer stored components than an input declares read as GL's `(0, 0, 0, 1)` defaults.
pub fn vertex_buffer_layouts(
    inputs: &[VertexInput],
) -> Result<[VertexBufferLayout; 2], VertexLayoutError> {
    let mut geometry = Vec::new();
    let mut constant = Vec::new();
    for input in inputs {
        let unsupported = || VertexLayoutError::UnsupportedType {
            name: input.name.clone(),
            ty: input.ty,
        };
        let (class, size) = class_and_size(input.ty).ok_or_else(unsupported)?;
        match stored_format(&input.attribute, class).ok_or_else(unsupported)? {
            Some((word, format)) => geometry.push(GpuVertexAttribute {
                format,
                offset: u64::from(word) * 4,
                shader_location: input.location,
            }),
            None => {
                let (word, format) = constant_format(&input.attribute, class, size);
                constant.push(GpuVertexAttribute {
                    format,
                    offset: u64::from(word) * 4,
                    shader_location: input.location,
                });
            }
        }
    }
    Ok([
        VertexBufferLayout {
            array_stride: IRIS_VERTEX_STRIDE,
            step_mode: VertexStepMode::Vertex,
            attributes: geometry,
        },
        VertexBufferLayout {
            array_stride: 0,
            step_mode: VertexStepMode::Vertex,
            attributes: constant,
        },
    ])
}

/// Replaces the expansion kernels' layout placeholders with this module's words.
pub(crate) fn substitute_layout_constants(source: &str) -> String {
    let mut source = source.replace("IRIS_VERTEX_WORDS", &format!("{IRIS_VERTEX_WORDS}u"));
    for field in STORED_FIELDS {
        source = source.replace(field.placeholder, &format!("{}u", field.word));
    }
    source
}

#[cfg(test)]
#[path = "vertex_tests.rs"]
mod tests;
