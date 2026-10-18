//! Terrain draws whose vertex stage reads Cinnabar's packed arenas (`geometry::pull`): the
//! wrapped vertex module, its pipeline layout (the program's groups, with the arena bindings
//! appended to group 3) and the group-3 bind group over the frame's arenas.

use std::sync::Arc;

use wgpu::{BindGroupLayoutEntry, BindingType, BufferBindingType, ShaderStages};

use super::draws::{DrawKind, GeometryStage};
use super::geometry::PulledArena;
use super::geometry::pull::{PulledResource, pull_requested, pulled_vertex_stage};
use super::loader::GpuProgram;
use shaderpack::StageKind;

/// The bindings a pulled draw adds to its program's group 3.
pub(crate) struct PulledBindings {
    /// The program's group-3 entries followed by the arena bindings.
    pub layout: wgpu::BindGroupLayout,
    pub resources: Vec<(u32, PulledResource)>,
}

impl PulledBindings {
    /// The arena entries of the group-3 bind group, or `None` while a resource is missing.
    pub(crate) fn entries<'a>(
        &self,
        arena: &'a PulledArena,
    ) -> Option<Vec<wgpu::BindGroupEntry<'a>>> {
        self.resources
            .iter()
            .map(|&(binding, resource)| {
                Some(wgpu::BindGroupEntry {
                    binding,
                    resource: arena.buffer(resource)?.as_entire_binding(),
                })
            })
            .collect()
    }
}

/// A program's vertex stage wrapped to read the arenas.
pub(crate) struct PulledProgram {
    pub module: wgpu::ShaderModule,
    pub layout: wgpu::PipelineLayout,
    pub bindings: Arc<PulledBindings>,
}

/// Storage buffers bound to the vertex stage by `entries`.
fn vertex_storage_buffers(entries: &[Vec<BindGroupLayoutEntry>; 4]) -> u32 {
    entries
        .iter()
        .flatten()
        .filter(|entry| {
            entry.visibility.contains(ShaderStages::VERTEX)
                && matches!(
                    entry.ty,
                    BindingType::Buffer {
                        ty: BufferBindingType::Storage { .. },
                        ..
                    }
                )
        })
        .count() as u32
}

/// Wraps `stage`'s vertex stage for pulled terrain draws.
fn pulled_program(
    device: &wgpu::Device,
    stage: &GeometryStage,
    program: &GpuProgram,
) -> Result<PulledProgram, String> {
    let reflection = &program.reflection;
    let entry = reflection
        .entry_points
        .iter()
        .find(|(kind, _)| *kind == StageKind::Vertex)
        .map(|(_, name)| name.as_str())
        .ok_or("no vertex entry point")?;
    let first_binding = stage.entries[3]
        .iter()
        .map(|entry| entry.binding + 1)
        .max()
        .unwrap_or(0);
    let pulled = pulled_vertex_stage(
        &stage.vertex_ir,
        entry,
        &reflection.vertex_inputs,
        first_binding,
    )?;
    let storage = vertex_storage_buffers(&stage.entries) + pulled.storage_buffers();
    let limit = device.limits().max_storage_buffers_per_shader_stage;
    if storage > limit {
        return Err(format!(
            "the vertex stage would bind {storage} storage buffers, over the device's {limit}"
        ));
    }
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(&format!("shader pack {} pulled vsh", program.name)),
        source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(pulled.module.clone())),
    });
    let mut group3 = stage.entries[3].clone();
    group3.extend(pulled.layout_entries());
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(&format!("shader pack {} pulled group 3", program.name)),
        entries: &group3,
    });
    let groups = [
        program.layouts[0].as_ref(),
        program.layouts[1].as_ref(),
        program.layouts[2].as_ref(),
        Some(&layout),
    ];
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(&format!("shader pack {} pulled", program.name)),
        bind_group_layouts: &groups,
        immediate_size: 0,
    });
    if let Some(error) = bevy::tasks::block_on(scope.pop()) {
        return Err(format!(
            "the device rejects the pulled vertex stage: {error}"
        ));
    }
    Ok(PulledProgram {
        module,
        layout: pipeline_layout,
        bindings: Arc::new(PulledBindings {
            layout,
            resources: pulled.resources,
        }),
    })
}

/// Pulled vertex stages per geometry stage for the terrain draws of `selected`, when
/// `CINNABAR_SHADERPACK_PULL=1`. All or nothing: terrain pages hold either expanded vertices
/// or face refs, so one program that cannot be wrapped keeps every terrain draw expanded.
pub(crate) fn pulled_programs(
    device: &wgpu::Device,
    pack: &str,
    selected: &[(DrawKind, usize)],
    stages: &[GeometryStage],
    programs: &[GpuProgram],
) -> Vec<Option<PulledProgram>> {
    let none = || stages.iter().map(|_| None).collect();
    if !pull_requested() {
        return none();
    }
    let started = std::time::Instant::now();
    let mut pulled: Vec<Option<PulledProgram>> = none();
    for (kind, index) in selected {
        if kind.category().is_none() || pulled[*index].is_some() {
            continue;
        }
        let stage = &stages[*index];
        let program = &programs[stage.program];
        match pulled_program(device, stage, program) {
            Ok(wrapped) => pulled[*index] = Some(wrapped),
            Err(error) => {
                bevy::log::warn!(
                    target: "cinnabar::shaderpack",
                    pack, program = %program.name,
                    "terrain stays expanded: the vertex stage cannot read the arenas: {error}"
                );
                return none();
            }
        }
    }
    bevy::log::info!(
        target: "cinnabar::shaderpack",
        pack,
        programs = pulled.iter().flatten().count(),
        "terrain draws read the arenas directly; wrapped in {:.0?}", started.elapsed()
    );
    pulled
}
