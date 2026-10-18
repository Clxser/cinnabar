//! Vertex pulling: pack vertex stages that read Cinnabar's packed arenas directly instead of
//! expanded vertices. Opt-in with `CINNABAR_SHADERPACK_PULL=1` when a pack loads; expansion
//! stays the default because the full arena decode runs for every corner in every pass. At
//! render distance 16 on an RTX 4070 Laptop it saved about 550 MiB of VRAM but raised GPU frame
//! time from 9.5 to 15.1 ms in BSL (shadow 1.9 → 5.4 ms) and from 7.8 to 10.9 ms in Photon.
//!
//! - **Pages** hold one 16-byte face ref per face slot instead of four 44-byte vertices:
//!   `[record, origin slot, the job's covered block and reserved-face flags, slot | kind << 8]`
//!   (see [`section_face_refs`]), written from the planned jobs with no expansion dispatch.
//! - **Draws** are instanced: six indices of one quad per instance, `first_instance` the face,
//!   and the page bound as an instance-step vertex buffer at location 0
//!   ([`face_ref_buffer_layout`]). The vertex index is the face's corner.
//! - **The vertex stage** is the pack's, wrapped at the naga level ([`pulled_vertex_stage`]):
//!   its entry point becomes a plain function; the decode library (`pull.wesl` composed with
//!   the expansion modules, which share `cube_face`/`model_face`/`liquid_face` and
//!   `job_slot_vertex` with the kernels) is merged into the module with its bindings moved
//!   to group 3 after the pack's buffers; and a generated entry, [`PULLED_ENTRY`], decodes
//!   the face's corner, converts it to every input with the precision and defaults the
//!   expanded stream gives (so pixels match expansion), and calls the pack's function.

use std::sync::LazyLock;

use naga::{
    AddressSpace, ArraySize, Binding, Block, BuiltIn, Expression, Function, FunctionArgument,
    FunctionResult, Handle, Module, ResourceBinding, Scalar, ScalarKind, ShaderStage, Span,
    Statement, StructMember, Type, TypeInner, VectorSize,
};
use shaderpack::VertexInput;
use wgpu::{BindGroupLayoutEntry, BindingType, BufferBindingType, ShaderStages, VertexFormat};

use super::jobs::{JOB_KINDS, SectionPlan, job_slots};
use super::pipelines::expansion_shader_source;
use super::vertex::{
    BLOCK_ID, COLOR, CONSTANT_STREAM_WORDS, GEOMETRY_VERTEX_BUFFER, InputFetch, LIGHTMAP,
    MID_BLOCK, MID_TEX, NORMAL, POSITION, TANGENT, UV0, input_fetches,
};

/// Environment switch: `1` builds pulled terrain pipelines when a pack loads.
pub(crate) const PULL_ENV: &str = "CINNABAR_SHADERPACK_PULL";
/// Bytes of one face ref in a page.
pub(crate) const FACE_REF_BYTES: u64 = 16;
/// Size of a face-ref page: a million faces, so the last page wastes little.
pub(crate) const PULL_PAGE_BYTES: u64 = 16 << 20;
/// Name of the generated vertex entry point of a pulled stage.
pub(crate) const PULLED_ENTRY: &str = "cnb_pulled_main";
/// Vertex location of the face ref.
const FACE_REF_LOCATION: u32 = 0;

/// Whether pulled terrain was asked for.
pub(crate) fn pull_requested() -> bool {
    env_compat::var_os(PULL_ENV).is_some_and(|value| value == "1")
}

/// The instance-step vertex buffer layout of a pulled draw: the page's face refs.
pub(crate) fn face_ref_buffer_layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRIBUTES: [wgpu::VertexAttribute; 1] = [wgpu::VertexAttribute {
        format: VertexFormat::Uint32x4,
        offset: 0,
        shader_location: FACE_REF_LOCATION,
    }];
    wgpu::VertexBufferLayout {
        array_stride: FACE_REF_BYTES,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &ATTRIBUTES,
    }
}

/// The face refs of a placed section, in its face order: every slot each job reserved.
pub(crate) fn section_face_refs(plan: &SectionPlan, first_face: u32) -> Vec<[u32; 4]> {
    let mut refs = vec![[0; 4]; plan.faces as usize];
    for (kind, jobs) in JOB_KINDS.iter().zip(&plan.jobs) {
        for job in jobs {
            for slot in 0..job_slots(job[3]) {
                refs[(job[2] - first_face + slot) as usize] =
                    [job[0], job[1], job[3], slot | ((*kind as u32) << 8)];
            }
        }
    }
    refs
}

/// An arena resource the pulled vertex stage reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum PulledResource {
    Quads,
    Origins,
    Materials,
    BiomeRecords,
    BiomeTints,
    ModelTemplates,
    GeometryStreams,
    /// The world clock and biome query tables (uniform).
    BiomeQuery,
    /// The material attribute table, two words per material.
    Attributes,
    AtlasRemap,
    /// The expansion parameters (uniform): origin, `separateAo`, `oldLighting`, materials.
    Params,
}

impl PulledResource {
    const ALL: [Self; 11] = [
        Self::Quads,
        Self::Origins,
        Self::Materials,
        Self::BiomeRecords,
        Self::BiomeTints,
        Self::ModelTemplates,
        Self::GeometryStreams,
        Self::BiomeQuery,
        Self::Attributes,
        Self::AtlasRemap,
        Self::Params,
    ];

    /// The resource the decode library declares at `binding`: the expansion kernels' group 0
    /// (the packed-chunk bindings) and group 1 (their outputs).
    fn declared_at(binding: &ResourceBinding) -> Option<Self> {
        Some(match (binding.group, binding.binding) {
            (0, 1) => Self::Quads,
            (0, 2) => Self::Origins,
            (0, 3) => Self::Materials,
            (0, 7) => Self::BiomeRecords,
            (0, 8) => Self::BiomeTints,
            (0, 12) => Self::ModelTemplates,
            (0, 13) => Self::GeometryStreams,
            (0, b) if b == crate::material_shader::BIOME_QUERY_TABLES_BINDING => Self::BiomeQuery,
            (1, 0) => Self::Attributes,
            (1, 2) => Self::AtlasRemap,
            (1, 3) => Self::Params,
            _ => return None,
        })
    }

    pub(crate) fn uniform(self) -> bool {
        matches!(self, Self::BiomeQuery | Self::Params)
    }
}

/// A pack vertex stage wrapped to read the arenas.
pub(crate) struct PulledStage {
    pub(crate) module: Module,
    /// Group-3 bindings the stage adds, with the resource each binds.
    pub(crate) resources: Vec<(u32, PulledResource)>,
}

impl PulledStage {
    /// Layout entries of the added group-3 bindings, visible to the vertex stage.
    pub(crate) fn layout_entries(&self) -> impl Iterator<Item = BindGroupLayoutEntry> + '_ {
        self.resources
            .iter()
            .map(|&(binding, resource)| BindGroupLayoutEntry {
                binding,
                visibility: ShaderStages::VERTEX,
                ty: BindingType::Buffer {
                    ty: if resource.uniform() {
                        BufferBindingType::Uniform
                    } else {
                        BufferBindingType::Storage { read_only: true }
                    },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            })
    }

    pub(crate) fn storage_buffers(&self) -> u32 {
        self.resources.iter().filter(|(_, r)| !r.uniform()).count() as u32
    }
}

/// Wraps the vertex entry point `entry` of the translated `vertex` stage, whose inputs are
/// `inputs` (`ProgramReflection::vertex_inputs`), into [`PULLED_ENTRY`]; the decode
/// library's bindings take group 3 from `first_binding` on.
pub(crate) fn pulled_vertex_stage(
    vertex: &Module,
    entry: &str,
    inputs: &[VertexInput],
    first_binding: u32,
) -> Result<PulledStage, String> {
    let entry_index = vertex
        .entry_points
        .iter()
        .position(|e| e.stage == ShaderStage::Vertex && e.name == entry)
        .ok_or_else(|| format!("no vertex entry point `{entry}`"))?;
    let fetches = input_fetches(inputs).map_err(|e| e.to_string())?;
    let arguments = &vertex.entry_points[entry_index].function.arguments;
    let sources = arguments
        .iter()
        .map(|argument| {
            let (ty, size) = value_type(vertex, argument.ty)?;
            let source = match argument.binding {
                Some(Binding::Location { location, .. }) => fetches
                    .iter()
                    .find(|fetch| fetch.location == location)
                    .map(|fetch| fetch_value(fetch, size))
                    .ok_or_else(|| format!("input at location {location} is not reflected"))?,
                Some(Binding::BuiltIn(BuiltIn::VertexIndex)) => "instance * 4u + corner".into(),
                Some(Binding::BuiltIn(BuiltIn::InstanceIndex)) => "0u".into(),
                ref other => return Err(format!("unsupported vertex input {other:?}")),
            };
            Ok((ty, source))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut library = library_module(&sources)?;
    let resources = rebind_library(&mut library, first_binding);
    let mut module = vertex.clone();
    let merged = merge(&mut module, &library)?;
    let inputs_function = library
        .functions
        .iter()
        .find(|(_, f)| f.name.as_deref() == Some(INPUTS_FUNCTION))
        .map(|(handle, _)| merged.functions[handle.index()])
        .ok_or("the decode library lost its inputs function")?;
    wrap_entry(&mut module, entry_index, inputs_function)?;
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .map_err(|e| format!("pulled vertex stage fails validation: {e:?}"))?;
    Ok(PulledStage { module, resources })
}

const INPUTS_FUNCTION: &str = "cnb_pulled_inputs";
const PROBE_ENTRY: &str = "cnb_pull_probe";

/// The WGSL type and component count of a vertex input type.
fn value_type(module: &Module, ty: Handle<Type>) -> Result<(String, u32), String> {
    let scalar = |scalar: Scalar| match (scalar.kind, scalar.width) {
        (ScalarKind::Float, 4) => Ok("f32"),
        (ScalarKind::Sint, 4) => Ok("i32"),
        (ScalarKind::Uint, 4) => Ok("u32"),
        _ => Err(format!("unsupported input scalar {scalar:?}")),
    };
    match module.types[ty].inner {
        TypeInner::Scalar(s) => Ok((scalar(s)?.into(), 1)),
        TypeInner::Vector { size, scalar: s } => {
            Ok((format!("vec{}<{}>", size as u32, scalar(s)?), size as u32))
        }
        ref other => Err(format!("unsupported input type {other:?}")),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Float,
    Sint,
    Uint,
}

fn format_class(format: VertexFormat) -> Class {
    use VertexFormat as F;
    match format {
        F::Sint8
        | F::Sint8x2
        | F::Sint8x4
        | F::Sint16
        | F::Sint16x2
        | F::Sint16x4
        | F::Sint32
        | F::Sint32x2
        | F::Sint32x3
        | F::Sint32x4 => Class::Sint,
        F::Uint8
        | F::Uint8x2
        | F::Uint8x4
        | F::Uint16
        | F::Uint16x2
        | F::Uint16x4
        | F::Uint32
        | F::Uint32x2
        | F::Uint32x3
        | F::Uint32x4 => Class::Uint,
        _ => Class::Float,
    }
}

/// WGSL for the value one input reads, with `size` components: the decoded vertex `v`
/// stored and fetched as the expanded stream does (packed then unpacked in the stored
/// format, missing components as GL's `(0, 0, 0, 1)`), or the constant stream's words.
fn fetch_value(fetch: &InputFetch, size: u32) -> String {
    let full = if fetch.buffer == GEOMETRY_VERTEX_BUFFER {
        let stored = [
            (POSITION, "vec4<f32>(v.position, 1.0)"),
            (COLOR, "unpack4x8unorm(pack4x8unorm(v.colour))"),
            (
                UV0,
                "vec4<f32>(unpack2x16unorm(pack2x16unorm(v.uv)), 0.0, 1.0)",
            ),
            (
                MID_TEX,
                "vec4<f32>(unpack2x16unorm(pack2x16unorm(v.mid_uv)), 0.0, 1.0)",
            ),
            (
                LIGHTMAP,
                "vec4<f32>(unpack2x16float(pack2x16float(v.light)), 0.0, 1.0)",
            ),
            (
                NORMAL,
                "unpack4x8snorm(pack4x8snorm(vec4<f32>(v.normal, 0.0)))",
            ),
            (TANGENT, "unpack4x8snorm(pack4x8snorm(v.tangent))"),
            // Signed shorts and bytes, sign-extended as the vertex fetch does.
            (
                BLOCK_ID,
                "vec4<i32>((v.block << vec2(16u)) >> vec2(16u), 0, 1)",
            ),
            (
                MID_BLOCK,
                "(vec4<i32>(v.mid_block) << vec4(24u)) >> vec4(24u)",
            ),
        ];
        stored
            .iter()
            .find(|(field, _)| field.word == fetch.word)
            .map_or_else(|| "vec4<f32>(0.0)".to_owned(), |(_, value)| (*value).into())
    } else {
        let components = fetch.format.size() / 4;
        let class = format_class(fetch.format);
        let values: Vec<String> = (0..4_u64)
            .map(|index| {
                let word = (index < components)
                    .then(|| CONSTANT_STREAM_WORDS[(u64::from(fetch.word) + index) as usize]);
                let default = u32::from(index == 3);
                match class {
                    Class::Float => {
                        let value = word.map_or(default as f32, f32::from_bits);
                        format!("{value:?}")
                    }
                    Class::Sint => format!("{}i", word.unwrap_or(default) as i32),
                    Class::Uint => format!("{}u", word.unwrap_or(default)),
                }
            })
            .collect();
        let ty = match class {
            Class::Float => "f32",
            Class::Sint => "i32",
            Class::Uint => "u32",
        };
        format!("vec4<{ty}>({})", values.join(", "))
    };
    match size {
        1 => format!("({full}).x"),
        2 => format!("({full}).xy"),
        3 => format!("({full}).xyz"),
        _ => full,
    }
}

/// The decode library composed from `pull.wesl` and the expansion modules, as WGSL.
fn library_source() -> Result<&'static str, String> {
    static LIBRARY: LazyLock<Result<String, String>> = LazyLock::new(compose_library);
    LIBRARY.as_deref().map_err(Clone::clone)
}

fn compose_library() -> Result<String, String> {
    let mut resolver = wesl::resolver::VirtualResolver::new();
    let modules = crate::shader_source::composable_sources()
        .into_iter()
        .chain([
            (
                "render::shaderpack::geometry::expand_common",
                expansion_shader_source(include_str!("expand_common.wesl")),
            ),
            (
                "render::shaderpack::geometry::expand_cubes",
                expansion_shader_source(include_str!("expand_cubes.wesl")),
            ),
            (
                "render::shaderpack::geometry::expand_models",
                expansion_shader_source(include_str!("expand_models.wesl")),
            ),
            (
                "render::shaderpack::geometry::expand_liquids",
                expansion_shader_source(include_str!("expand_liquids.wesl")),
            ),
        ]);
    for (path, source) in modules {
        let path = path
            .parse()
            .map_err(|e| format!("module path {path}: {e:?}"))?;
        resolver.add_module(path, source.into());
    }
    let root: wesl::syntax::ModulePath = "render::shaderpack::geometry::pull"
        .parse()
        .map_err(|e| format!("root module path: {e:?}"))?;
    resolver.add_module(
        root.clone(),
        expansion_shader_source(include_str!("pull.wesl")).into(),
    );
    let options = wesl::CompileOptions {
        visibility: false,
        strip: true,
        keep: Some(vec!["cnb_pull_vertex".into()]),
        mangler: wesl::ManglerKind::Escape,
        ..Default::default()
    };
    wesl::compile(
        &root,
        &options,
        &crate::shader_source::FixtureResolver(resolver),
    )
    .map(|composed| composed.to_string())
    .map_err(|e| e.diagnostic().render_plain())
}

/// The decode library with a generated `cnb_pulled_inputs(face_ref, corner, instance)`
/// returning each of the pack entry's arguments (`(WGSL type, value)`), validated and
/// reduced to what that function needs.
fn library_module(arguments: &[(String, String)]) -> Result<Module, String> {
    let mut source = library_source()?.to_owned();
    source.push_str("\nstruct CnbPulledInputs {\n");
    for (index, (ty, _)) in arguments.iter().enumerate() {
        source.push_str(&format!("    a{index}: {ty},\n"));
    }
    if arguments.is_empty() {
        source.push_str("    unused: u32,\n");
    }
    source.push_str(&format!(
        "}}\nfn {INPUTS_FUNCTION}(face_ref: vec4<u32>, corner: u32, instance: u32) -> CnbPulledInputs {{\n    let v = cnb_pull_vertex(face_ref, corner);\n    var inputs: CnbPulledInputs;\n"
    ));
    for (index, (_, value)) in arguments.iter().enumerate() {
        source.push_str(&format!("    inputs.a{index} = {value};\n"));
    }
    source.push_str(&format!(
        "    return inputs;\n}}\n@vertex fn {PROBE_ENTRY}(@location(0) face_ref: vec4<u32>, @builtin(vertex_index) corner: u32, @builtin(instance_index) instance: u32) -> @builtin(position) vec4<f32> {{\n    _ = {INPUTS_FUNCTION}(face_ref, corner, instance);\n    return vec4(0.0);\n}}\n"
    ));
    let mut module = naga::front::wgsl::parse_str(&source)
        .map_err(|e| format!("decode library: {}", e.emit_to_string(&source)))?;
    // The decoders' modules each declare the bindings they read, some of them twice with
    // their own types (`chunk.wesl` and `chunk_bindings.wesl`), so every declaration gets a
    // binding of its own, which names its resource until `rebind_library` numbers them.
    let mut declared = [0_u32; PulledResource::ALL.len()];
    for (_, global) in module.global_variables.iter_mut() {
        let Some(binding) = &mut global.binding else {
            continue;
        };
        let resource = PulledResource::declared_at(binding).ok_or_else(|| {
            format!(
                "decode library binds unexpected resource {:?} at {binding:?}",
                global.name
            )
        })?;
        if let AddressSpace::Storage { access } = global.space
            && access.contains(naga::StorageAccess::STORE)
        {
            return Err(format!("decode library writes {:?}", global.name));
        }
        let index = resource as usize;
        *binding = ResourceBinding {
            group: 3,
            binding: (index * DECLARATIONS) as u32 + declared[index],
        };
        declared[index] += 1;
    }
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .map_err(|e| format!("decode library fails validation: {e:?}"))?;
    naga::compact::compact(&mut module, naga::compact::KeepUnused::No);
    module.entry_points.clear();
    Ok(module)
}

/// Room in the provisional binding numbers for declarations of one resource.
const DECLARATIONS: usize = 16;

/// Numbers the library's remaining bindings in group 3 from `first_binding`, in
/// [`PulledResource`] order, returning what each binds; a resource two declarations read is
/// bound twice.
fn rebind_library(library: &mut Module, first_binding: u32) -> Vec<(u32, PulledResource)> {
    let mut bound: Vec<_> = library
        .global_variables
        .iter()
        .filter_map(|(handle, global)| Some((global.binding.as_ref()?.binding, handle)))
        .collect();
    bound.sort();
    bound
        .into_iter()
        .enumerate()
        .map(|(index, (provisional, handle))| {
            let binding = first_binding + index as u32;
            library.global_variables.get_mut(handle).binding =
                Some(ResourceBinding { group: 3, binding });
            (
                binding,
                PulledResource::ALL[provisional as usize / DECLARATIONS],
            )
        })
        .collect()
}

/// Handles of the library's items in the module it was merged into, by library index.
struct Merged {
    types: Vec<Handle<Type>>,
    constants: Vec<Option<Handle<naga::Constant>>>,
    expressions: Vec<Handle<Expression>>,
    globals: Vec<Handle<naga::GlobalVariable>>,
    functions: Vec<Handle<Function>>,
}

impl Merged {
    fn ty(&self, handle: Handle<Type>) -> Handle<Type> {
        self.types[handle.index()]
    }

    fn constant(&self, handle: Handle<naga::Constant>) -> Result<Handle<naga::Constant>, String> {
        self.constants[handle.index()].ok_or_else(|| "constant used before its definition".into())
    }
}

/// Appends `library`'s types, constants, globals and functions (not its entry points) to
/// `module`. The library has no overrides and no diagnostic filters.
fn merge(module: &mut Module, library: &Module) -> Result<Merged, String> {
    if !library.overrides.is_empty() {
        return Err("decode library declares overrides".into());
    }
    let mut merged = Merged {
        types: Vec::with_capacity(library.types.len()),
        constants: vec![None; library.constants.len()],
        expressions: Vec::with_capacity(library.global_expressions.len()),
        globals: Vec::with_capacity(library.global_variables.len()),
        functions: Vec::with_capacity(library.functions.len()),
    };
    for (handle, ty) in library.types.iter() {
        let inner = match ty.inner {
            TypeInner::Pointer { base, space } => TypeInner::Pointer {
                base: merged.ty(base),
                space,
            },
            TypeInner::Array { base, size, stride } => TypeInner::Array {
                base: merged.ty(base),
                size: match size {
                    ArraySize::Pending(_) => return Err("override-sized array".into()),
                    size => size,
                },
                stride,
            },
            TypeInner::BindingArray { .. } => return Err("binding array".into()),
            TypeInner::Struct { ref members, span } => TypeInner::Struct {
                members: members
                    .iter()
                    .map(|member| StructMember {
                        ty: merged.ty(member.ty),
                        ..member.clone()
                    })
                    .collect(),
                span,
            },
            ref inner => inner.clone(),
        };
        let mapped = module.types.insert(
            Type {
                name: ty.name.clone(),
                inner,
            },
            library.types.get_span(handle),
        );
        merged.types.push(mapped);
    }
    for (key, ty) in &library.special_types.predeclared_types {
        let ty = merged.ty(*ty);
        module
            .special_types
            .predeclared_types
            .entry(key.clone())
            .or_insert(ty);
    }
    let map_constant = |module: &mut Module,
                        merged: &mut Merged,
                        handle: Handle<naga::Constant>|
     -> Handle<naga::Constant> {
        if let Some(mapped) = merged.constants[handle.index()] {
            return mapped;
        }
        let constant = &library.constants[handle];
        let mapped = module.constants.append(
            naga::Constant {
                name: constant.name.clone(),
                ty: merged.ty(constant.ty),
                init: merged.expressions[constant.init.index()],
            },
            library.constants.get_span(handle),
        );
        merged.constants[handle.index()] = Some(mapped);
        mapped
    };
    for (handle, expression) in library.global_expressions.iter() {
        let mapped = match *expression {
            Expression::Literal(literal) => Expression::Literal(literal),
            Expression::Constant(constant) => {
                Expression::Constant(map_constant(module, &mut merged, constant))
            }
            Expression::ZeroValue(ty) => Expression::ZeroValue(merged.ty(ty)),
            Expression::Compose { ty, ref components } => Expression::Compose {
                ty: merged.ty(ty),
                components: components
                    .iter()
                    .map(|c| merged.expressions[c.index()])
                    .collect(),
            },
            Expression::Splat { size, value } => Expression::Splat {
                size,
                value: merged.expressions[value.index()],
            },
            ref other => return Err(format!("unsupported constant expression {other:?}")),
        };
        let mapped = module
            .global_expressions
            .append(mapped, library.global_expressions.get_span(handle));
        merged.expressions.push(mapped);
    }
    for (handle, _) in library.constants.iter() {
        map_constant(module, &mut merged, handle);
    }
    for (handle, global) in library.global_variables.iter() {
        let mapped = module.global_variables.append(
            naga::GlobalVariable {
                ty: merged.ty(global.ty),
                init: global.init.map(|init| merged.expressions[init.index()]),
                ..global.clone()
            },
            library.global_variables.get_span(handle),
        );
        merged.globals.push(mapped);
    }
    for (handle, function) in library.functions.iter() {
        let mut function = function.clone();
        for argument in &mut function.arguments {
            argument.ty = merged.ty(argument.ty);
        }
        if let Some(result) = &mut function.result {
            result.ty = merged.ty(result.ty);
        }
        for (_, local) in function.local_variables.iter_mut() {
            local.ty = merged.ty(local.ty);
        }
        for (_, expression) in function.expressions.iter_mut() {
            match expression {
                Expression::Constant(constant) => *constant = merged.constant(*constant)?,
                Expression::ZeroValue(ty)
                | Expression::Compose { ty, .. }
                | Expression::AtomicResult { ty, .. }
                | Expression::WorkGroupUniformLoadResult { ty }
                | Expression::SubgroupOperationResult { ty } => *ty = merged.ty(*ty),
                Expression::GlobalVariable(global) => *global = merged.globals[global.index()],
                Expression::CallResult(callee) => *callee = merged.functions[callee.index()],
                Expression::Override(_)
                | Expression::RayQueryVertexPositions { .. }
                | Expression::RayQueryProceedResult
                | Expression::RayQueryGetIntersection { .. }
                | Expression::CooperativeLoad { .. }
                | Expression::CooperativeMultiplyAdd { .. } => {
                    return Err(format!("unsupported expression {expression:?}"));
                }
                _ => {}
            }
        }
        remap_calls(&mut function.body, &merged.functions)?;
        function.diagnostic_filter_leaf = None;
        let mapped = module
            .functions
            .append(function, library.functions.get_span(handle));
        merged.functions.push(mapped);
    }
    Ok(merged)
}

/// Points every call in `block` at the merged function.
fn remap_calls(block: &mut Block, functions: &[Handle<Function>]) -> Result<(), String> {
    for statement in block.iter_mut() {
        match statement {
            Statement::Block(inner) => remap_calls(inner, functions)?,
            Statement::If { accept, reject, .. } => {
                remap_calls(accept, functions)?;
                remap_calls(reject, functions)?;
            }
            Statement::Switch { cases, .. } => {
                for case in cases {
                    remap_calls(&mut case.body, functions)?;
                }
            }
            Statement::Loop {
                body, continuing, ..
            } => {
                remap_calls(body, functions)?;
                remap_calls(continuing, functions)?;
            }
            Statement::Call { function, .. } => *function = functions[function.index()],
            Statement::RayQuery { .. }
            | Statement::RayPipelineFunction(_)
            | Statement::CooperativeStore { .. } => {
                return Err(format!("unsupported statement {statement:?}"));
            }
            _ => {}
        }
    }
    Ok(())
}

/// Turns entry point `entry_index` into a plain function and adds [`PULLED_ENTRY`], which
/// passes it the values `inputs` (the merged `cnb_pulled_inputs`) returns, in argument order.
fn wrap_entry(
    module: &mut Module,
    entry_index: usize,
    inputs: Handle<Function>,
) -> Result<(), String> {
    let entry = module.entry_points.remove(entry_index);
    let mut pack = entry.function.clone();
    pack.name = Some("cnb_pack_vertex".into());
    for argument in &mut pack.arguments {
        argument.binding = None;
    }
    let result = pack.result.as_mut().map(|result| FunctionResult {
        ty: result.ty,
        binding: result.binding.take(),
    });
    let arguments = pack.arguments.len() as u32;
    let pack = module.functions.append(pack, Span::UNDEFINED);
    let u32_ty = module.types.insert(
        Type {
            name: None,
            inner: TypeInner::Scalar(Scalar::U32),
        },
        Span::UNDEFINED,
    );
    let ref_ty = module.types.insert(
        Type {
            name: None,
            inner: TypeInner::Vector {
                size: VectorSize::Quad,
                scalar: Scalar::U32,
            },
        },
        Span::UNDEFINED,
    );
    // Face ref, vertex index (the corner), instance index (the face).
    let argument = |ty, binding| FunctionArgument {
        name: None,
        ty,
        binding: Some(binding),
    };
    let mut function = Function {
        name: Some(PULLED_ENTRY.into()),
        arguments: vec![
            argument(
                ref_ty,
                Binding::Location {
                    location: FACE_REF_LOCATION,
                    interpolation: None,
                    sampling: None,
                    blend_src: None,
                    per_primitive: false,
                },
            ),
            argument(u32_ty, Binding::BuiltIn(BuiltIn::VertexIndex)),
            argument(u32_ty, Binding::BuiltIn(BuiltIn::InstanceIndex)),
        ],
        result,
        ..Function::default()
    };
    let expressions = &mut function.expressions;
    let call_arguments: Vec<_> = (0..3)
        .map(|index| expressions.append(Expression::FunctionArgument(index), Span::UNDEFINED))
        .collect();
    let values = expressions.append(Expression::CallResult(inputs), Span::UNDEFINED);
    let start = expressions.len();
    let fields: Vec<_> = (0..arguments)
        .map(|index| {
            expressions.append(
                Expression::AccessIndex {
                    base: values,
                    index,
                },
                Span::UNDEFINED,
            )
        })
        .collect();
    let emitted = expressions.range_from(start);
    let output = function.result.is_some().then(|| {
        function
            .expressions
            .append(Expression::CallResult(pack), Span::UNDEFINED)
    });
    let body = &mut function.body;
    body.push(
        Statement::Call {
            function: inputs,
            arguments: call_arguments,
            result: Some(values),
        },
        Span::UNDEFINED,
    );
    if !fields.is_empty() {
        body.push(Statement::Emit(emitted), Span::UNDEFINED);
    }
    body.push(
        Statement::Call {
            function: pack,
            arguments: fields,
            result: output,
        },
        Span::UNDEFINED,
    );
    body.push(Statement::Return { value: output }, Span::UNDEFINED);
    module.entry_points.push(naga::EntryPoint {
        name: PULLED_ENTRY.into(),
        function,
        ..entry
    });
    Ok(())
}

#[cfg(test)]
#[path = "pull_tests.rs"]
mod tests;
