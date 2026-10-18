//! The pulled vertex stage's generated wrapper, without a GPU.

use super::*;
use crate::shaderpack::geometry::jobs::{JOB_OVERLAY_FACE, JOB_REVERSE_FACE, JobKind};

fn terrain_probe() -> shaderpack::ProgramTranslation {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/shaderpacks/terrain_probe");
    let pack = shaderpack::ShaderPack::open(&path).expect("fixture pack opens");
    pack.translate(&shaderpack::TranslateConfig::default())
        .programs
        .into_iter()
        .find(|program| program.program.name == "gbuffers_terrain")
        .expect("fixture defines gbuffers_terrain")
}

/// The fixture's vertex stage wrapped with bindings from 4 on.
fn wrapped_probe() -> (shaderpack::ProgramTranslation, PulledStage) {
    let program = terrain_probe();
    let reflection = program.reflection.as_ref().expect("reflection");
    let vertex = program
        .stages
        .iter()
        .find(|stage| stage.stage == shaderpack::StageKind::Vertex)
        .and_then(|stage| stage.result.as_ref().ok())
        .expect("vertex stage compiled");
    let entry = reflection
        .entry_points
        .iter()
        .find(|(stage, _)| *stage == shaderpack::StageKind::Vertex)
        .map(|(_, name)| name.clone())
        .expect("vertex entry point");
    let stage = pulled_vertex_stage(&vertex.module, &entry, &reflection.vertex_inputs, 4)
        .unwrap_or_else(|error| panic!("{error}"));
    (program, stage)
}

#[test]
fn the_decode_library_composes_from_the_expansion_modules() {
    let source = library_source().unwrap_or_else(|error| panic!("{error}"));
    assert!(
        source.contains("fn cnb_pull_vertex("),
        "the root keeps its name"
    );
    assert!(
        !source.contains("@compute"),
        "the kernels' entry points are stripped"
    );
}

#[test]
fn a_wrapped_pack_vertex_stage_reads_the_arenas_in_group_three() {
    let (program, stage) = wrapped_probe();
    let module = &stage.module;
    let entries: Vec<_> = module
        .entry_points
        .iter()
        .map(|e| e.name.as_str())
        .collect();
    assert_eq!(
        entries,
        [PULLED_ENTRY],
        "the pack's entry point became a function"
    );
    assert!(
        module
            .functions
            .iter()
            .any(|(_, f)| f.name.as_deref() == Some("cnb_pack_vertex"))
    );
    let entry = &module.entry_points[0].function;
    let face_ref = &entry.arguments[0];
    assert!(matches!(
        face_ref.binding,
        Some(Binding::Location {
            location: FACE_REF_LOCATION,
            ..
        })
    ));
    // The pack's own bindings keep their groups; the decode's take group 3 from 4 on, one
    // binding per resource, read-only.
    let bindings: Vec<_> = stage
        .resources
        .iter()
        .map(|(binding, _)| *binding)
        .collect();
    assert_eq!(bindings, (4..4 + bindings.len() as u32).collect::<Vec<_>>());
    let resources: Vec<_> = stage.resources.iter().map(|(_, r)| *r).collect();
    for needed in [
        PulledResource::Quads,
        PulledResource::Origins,
        PulledResource::Materials,
        PulledResource::GeometryStreams,
        PulledResource::AtlasRemap,
        PulledResource::Params,
        PulledResource::Attributes,
    ] {
        assert!(resources.contains(&needed), "{needed:?} in {resources:?}");
    }
    // Desktop adapters bind far more; the decoders read two resources through two
    // declarations each.
    assert!(stage.storage_buffers() <= 12, "{resources:?}");
    for (index, resource) in PulledResource::ALL.iter().enumerate() {
        assert_eq!(*resource as usize, index, "ALL lists resources in order");
    }
    let pack_globals = program
        .stages
        .iter()
        .find(|s| s.stage == shaderpack::StageKind::Vertex)
        .and_then(|s| s.result.as_ref().ok())
        .map(|compiled| compiled.module.global_variables.len())
        .expect("vertex stage");
    let added: Vec<_> = module
        .global_variables
        .iter()
        .skip(pack_globals)
        .filter_map(|(_, global)| global.binding.clone())
        .collect();
    assert!(!added.is_empty());
    assert!(
        added
            .iter()
            .all(|binding| binding.group == 3 && binding.binding >= 4)
    );
    for (_, global) in module.global_variables.iter() {
        if let AddressSpace::Storage { access } = global.space {
            assert!(!access.contains(naga::StorageAccess::STORE), "{global:?}");
        }
    }
}

#[test]
fn every_reserved_slot_gets_a_face_ref_naming_its_job_and_kernel() {
    let mut plan = SectionPlan {
        faces: 4,
        ..SectionPlan::default()
    };
    let flags = JOB_OVERLAY_FACE | JOB_REVERSE_FACE | 0x0102;
    plan.jobs[JobKind::Cube as usize].push([7, 3, 100, flags]);
    plan.jobs[JobKind::Liquid as usize].push([9, 3, 103, 0]);
    let refs = section_face_refs(&plan, 100);
    let cube = JobKind::Cube as u32;
    let liquid = JobKind::Liquid as u32;
    assert_eq!(
        refs,
        [
            [7, 3, flags, cube << 8],
            [7, 3, flags, 1 | (cube << 8)],
            [7, 3, flags, 2 | (cube << 8)],
            [9, 3, 0, liquid << 8],
        ]
    );
}

#[test]
fn constant_inputs_read_the_constant_streams_words() {
    use super::super::vertex::CONSTANT_VERTEX_BUFFER;
    let fetch = |word, format| InputFetch {
        location: 0,
        buffer: CONSTANT_VERTEX_BUFFER,
        word,
        format,
    };
    // Chunk fade: one float, then GL's defaults.
    assert_eq!(
        fetch_value(&fetch(8, VertexFormat::Float32), 4),
        "vec4<f32>(1.0, 0.0, 0.0, 1.0)"
    );
    // Overlay as integers: Java's NO_OVERLAY (0, 10).
    assert_eq!(
        fetch_value(&fetch(9, VertexFormat::Sint32x2), 2),
        "(vec4<i32>(0i, 10i, 0i, 1i)).xy"
    );
    assert_eq!(
        fetch_value(&fetch(0, VertexFormat::Float32x3), 3),
        "(vec4<f32>(0.0, 0.0, 0.0, 1.0)).xyz"
    );
}
