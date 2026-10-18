//! GLSL that OpenGL drivers accept and naga's parser needs rewritten: interface blocks
//! between stages, bools in buffer blocks, integer 1D samplers and returns that convert.

use crate::{program, reflection};
use shaderpack::{PackSource, SampleType, ShaderPack, TextureDimension, TranslateConfig};

fn translate(files: &[(&str, &str)]) -> shaderpack::PackTranslation {
    ShaderPack::new(PackSource::from_files(files.iter().copied()))
        .translate(&TranslateConfig::default())
}

#[test]
fn interface_blocks_link_by_block_and_member_name() {
    let t = translate(&[
        (
            "composite.vsh",
            "#version 430 compatibility\n\
             out DATA { vec4 color; flat int id; mat3 m; } data_out;\n\
             void main() {\n\
                 gl_Position = ftransform();\n\
                 data_out.color = gl_Color;\n\
                 data_out.id = 3;\n\
                 data_out.m = mat3(2.0);\n\
             }\n",
        ),
        (
            "composite.fsh",
            "#version 430\n\
             in DATA { vec4 color; flat int id; mat3 m; } data_in;\n\
             /* RENDERTARGETS: 0 */\n\
             layout(location = 0) out vec4 colour;\n\
             void main() { colour = data_in.color * float(data_in.id) * data_in.m[0].x; }\n",
        ),
    ]);
    let pass = program(&t, "composite");
    reflection(pass);
    // The vertex stage writes and the fragment stage reads the same locations.
    let wgsl_like: Vec<String> = pass
        .stages
        .iter()
        .map(|s| {
            s.result
                .as_ref()
                .map(|c| c.glsl.clone())
                .unwrap_or_default()
        })
        .collect();
    for stage in &wgsl_like {
        assert!(stage.contains("iris_b_DATA_color_0"), "{stage}");
        assert!(stage.contains("flat"), "integer members are flat: {stage}");
    }
}

#[test]
fn buffer_block_bools_are_integers_read_and_written_through_conversions() {
    let t = translate(&[(
        "begin.csh",
        "#version 430\n\
         layout(local_size_x = 1) in;\n\
         const ivec3 workGroups = ivec3(1, 1, 1);\n\
         layout(std430, binding = 4) buffer State { vec3 v; bool idle; bool check; bvec2 pair; };\n\
         void main() {\n\
             idle = check;\n\
             check = v.x > 0.0 && !idle;\n\
             if (pair.x) { pair = bvec2(idle, true); }\n\
         }\n",
    )]);
    let begin = program(&t, "begin");
    let r = reflection(begin);
    // std430: vec3 at 0, the bools as 4-byte integers at 12 and 16, the pair at 24.
    assert_eq!(r.storage_buffers[0].min_size, 32);
}

#[test]
fn integer_1d_samplers_and_returns_that_convert_translate() {
    let t = translate(&[
        (
            "shaders.properties",
            "image.blocks = texBlockData RG_INTEGER RG32UI UNSIGNED_INT false false 64\n",
        ),
        (
            "composite.vsh",
            "#version 430 compatibility\nvoid main() { gl_Position = ftransform(); }\n",
        ),
        (
            "composite.fsh",
            "#version 430\n\
             uniform usampler1D texBlockData;\n\
             uint block(int i) { if (i < 0) return 0; return texelFetch(texBlockData, i, 0).r; }\n\
             float half_of(int i) { return i / 2; }\n\
             /* RENDERTARGETS: 0 */\n\
             layout(location = 0) out vec4 colour;\n\
             void main() { colour = vec4(float(block(1)), half_of(3), 0.0, 1.0); }\n",
        ),
    ]);
    let r = reflection(program(&t, "composite"));
    let blocks = r.textures.iter().find(|t| t.name == "texBlockData");
    assert_eq!(
        blocks.map(|t| (t.dimension, t.sample_type)),
        Some((TextureDimension::D1, SampleType::Uint))
    );
}
