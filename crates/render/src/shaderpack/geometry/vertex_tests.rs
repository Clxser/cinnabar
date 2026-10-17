use super::*;

fn input(location: u32, name: &str, ty: ValueType) -> VertexInput {
    VertexInput {
        location,
        name: name.into(),
        attribute: VertexAttribute::from_glsl_name(name),
        ty,
    }
}

fn vector(size: u8, scalar: ScalarType) -> ValueType {
    ValueType::Vector { size, scalar }
}

fn attribute(layout: &VertexBufferLayout, location: u32) -> (VertexFormat, u64) {
    let attribute = layout
        .attributes
        .iter()
        .find(|attribute| attribute.shader_location == location)
        .unwrap_or_else(|| panic!("location {location} is bound"));
    (attribute.format, attribute.offset)
}

fn constant_word(offset: u64) -> u32 {
    CONSTANT_STREAM_WORDS[(offset / 4) as usize]
}

#[test]
fn compatibility_inputs_read_their_stored_fields() {
    let float = ScalarType::Float;
    let inputs = [
        input(0, "iris_Vertex", vector(4, float)),
        input(1, "iris_Color", vector(4, float)),
        input(2, "iris_Normal", vector(3, float)),
        input(3, "iris_MultiTexCoord0", vector(4, float)),
        input(4, "iris_MultiTexCoord1", vector(4, float)),
        input(8, "mc_Entity", vector(4, float)),
        input(9, "mc_midTexCoord", vector(2, float)),
        input(10, "at_tangent", vector(4, float)),
        input(11, "at_midBlock", vector(4, float)),
    ];
    let [geometry, constant] = vertex_buffer_layouts(&inputs).unwrap();
    assert_eq!(geometry.array_stride, IRIS_VERTEX_STRIDE);
    assert!(constant.attributes.is_empty());
    let expected = [
        (0, POSITION),
        (1, COLOR),
        (2, NORMAL),
        (3, UV0),
        (4, LIGHTMAP_FLOAT),
        (8, BLOCK_ID),
        (9, MID_TEX),
        (10, TANGENT),
        (11, MID_BLOCK),
    ];
    for (location, field) in expected {
        assert_eq!(
            attribute(&geometry, location),
            (field.format, u64::from(field.word) * 4)
        );
    }
}

#[test]
fn core_profile_integer_lightmap_reads_the_rounded_field() {
    let inputs = [
        input(4, "vaUV2", vector(2, ScalarType::Int)),
        input(5, "iris_MultiTexCoord2", vector(4, ScalarType::Float)),
    ];
    let [geometry, _] = vertex_buffer_layouts(&inputs).unwrap();
    assert_eq!(
        attribute(&geometry, 4),
        (VertexFormat::Sint16x2, u64::from(LIGHTMAP_INT.word) * 4)
    );
    assert_eq!(
        attribute(&geometry, 5),
        (LIGHTMAP_FLOAT.format, u64::from(LIGHTMAP_FLOAT.word) * 4),
        "gl_MultiTexCoord2 is a second name for the lightmap"
    );
}

#[test]
fn attributes_terrain_lacks_read_iris_constants() {
    let inputs = [
        input(13, "mc_chunkFade", ValueType::Scalar(ScalarType::Float)),
        input(7, "vaUV1", vector(2, ScalarType::Int)),
        input(12, "at_velocity", vector(3, ScalarType::Float)),
        input(15, "myCustomInput", vector(4, ScalarType::Uint)),
    ];
    let [geometry, constant] = vertex_buffer_layouts(&inputs).unwrap();
    assert!(geometry.attributes.is_empty());
    assert_eq!(
        constant.array_stride, 0,
        "every vertex reads the same constants"
    );
    let (format, offset) = attribute(&constant, 13);
    assert_eq!(format, VertexFormat::Float32);
    assert_eq!(
        f32::from_bits(constant_word(offset)),
        1.0,
        "chunks are fully faded in"
    );
    let (format, offset) = attribute(&constant, 7);
    assert_eq!(format, VertexFormat::Sint32x2);
    assert_eq!(
        [constant_word(offset), constant_word(offset + 4)],
        [0, 10],
        "OverlayTexture.NO_OVERLAY"
    );
    let (format, offset) = attribute(&constant, 12);
    assert_eq!(format, VertexFormat::Float32x3);
    assert_eq!(
        (0..3).map(|i| constant_word(offset + i * 4)).sum::<u32>(),
        0
    );
    let (format, offset) = attribute(&constant, 15);
    assert_eq!(format, VertexFormat::Uint32x4);
    assert_eq!(
        (0..4).map(|i| constant_word(offset + i * 4)).sum::<u32>(),
        0
    );
}

#[test]
fn inputs_of_types_terrain_cannot_supply_are_rejected() {
    let matrix = ValueType::Matrix {
        columns: 4,
        rows: 4,
        scalar: ScalarType::Float,
    };
    let error = vertex_buffer_layouts(&[input(0, "iris_Vertex", matrix)]).unwrap_err();
    assert_eq!(
        error,
        VertexLayoutError::UnsupportedType {
            name: "iris_Vertex".into(),
            ty: matrix
        }
    );
    assert!(vertex_buffer_layouts(&[input(1, "iris_Color", vector(4, ScalarType::Int))]).is_err());
}

#[test]
fn every_layout_placeholder_is_resolved() {
    let source = "IRIS_VERTEX_WORDS IRIS_WORD_POSITION IRIS_WORD_COLOR IRIS_WORD_UV0 \
        IRIS_WORD_MID_TEX IRIS_WORD_LIGHTMAP_FLOAT IRIS_WORD_LIGHTMAP_INT IRIS_WORD_NORMAL \
        IRIS_WORD_TANGENT IRIS_WORD_BLOCK_ID IRIS_WORD_MID_BLOCK";
    let resolved = substitute_layout_constants(source);
    assert!(!resolved.contains("IRIS_"), "{resolved}");
    assert!(resolved.starts_with(&format!("{IRIS_VERTEX_WORDS}u 0u 3u")));
}
