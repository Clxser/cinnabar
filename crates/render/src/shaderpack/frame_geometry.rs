//! Recording the pack's geometry: the shadow pass, the sky, opaque and translucent terrain,
//! the depth copies Iris's `depthtex*`/`shadowtex*` semantics need, and the Vanilla overlay
//! that stands in for geometry the host does not draw yet (entities, particles, clouds).

use super::draws::DrawKind;
use super::geometry::{CONSTANT_VERTEX_BUFFER, GEOMETRY_VERTEX_BUFFER, GeometryCategory};
use super::loader::PackGpu;
use super::passes::{ViewState, clear_attachment, load_attachment, pass};
use super::sky::{SkyGeometry, SkyInputs, sky_geometry};
use super::targets::AtlasViews;
use bevy::render::render_phase::TrackedRenderPass;
use bevy::render::renderer::RenderDevice;
use wgpu::{CommandEncoder, Device, Queue, TextureView};

/// Terrain the pack's geometry programs draw.
pub(crate) trait TerrainDraws {
    /// Records `category` into `pass`, which holds the draw's pipeline and bind groups:
    /// the camera's visible sections, or every resident section for the shadow pass.
    /// Returns the draws issued.
    fn draw<'w>(
        &'w self,
        pass: &mut TrackedRenderPass<'w>,
        category: GeometryCategory,
        shadow: bool,
    ) -> u32;
}

/// What a view supplies for the pack's geometry this frame.
pub(crate) struct FrameGeometry<'a> {
    pub device: &'a RenderDevice,
    pub terrain: &'a dyn TerrainDraws,
    /// The block atlases and their generation, once built.
    pub atlas: Option<(AtlasViews<'a>, u64)>,
    pub sky: SkyInputs,
}

fn depth_attachment(
    view: &TextureView,
    clear: bool,
) -> Option<wgpu::RenderPassDepthStencilAttachment<'_>> {
    Some(wgpu::RenderPassDepthStencilAttachment {
        view,
        depth_ops: Some(wgpu::Operations {
            load: if clear {
                wgpu::LoadOp::Clear(1.0)
            } else {
                wgpu::LoadOp::Load
            },
            store: wgpu::StoreOp::Store,
        }),
        stencil_ops: None,
    })
}

/// Clears a depth attachment to the far plane.
fn clear_depth(encoder: &mut CommandEncoder, view: &TextureView) {
    drop(pass(
        encoder,
        "shader pack depth clear",
        &[],
        depth_attachment(view, true),
    ));
}

/// Records every draw of `kind` (at most one), each in its own render pass over its
/// outputs: `colortex` sides per `write_alt`, or `shadowcolor` targets in the shadow pass.
/// Draws whose outputs differ in size from the depth attachment are skipped (logged once).
fn record_kind(
    encoder: &mut CommandEncoder,
    state: &mut ViewState,
    geometry: &FrameGeometry<'_>,
    kind: DrawKind,
    write_alt: u32,
    depth: Option<(&TextureView, [u32; 2])>,
    sky: Option<&SkyGeometry>,
) {
    let pack = std::sync::Arc::clone(&state.pack);
    let Some(index) = pack.draws.iter().position(|d| d.kind == kind) else {
        return;
    };
    let draw = &pack.draws[index];
    let targets = &state.targets;
    let mut size = depth.map(|(_, size)| size);
    let mut consistent = true;
    let attachments: Vec<_> = draw
        .outputs
        .iter()
        .map(|output| {
            let buffer = (*output)?;
            let (view, extent) = if kind.shadow() {
                let shadow = pack.shadow.as_ref()?;
                let (.., slot, _) = shadow.slots.color.iter().find(|(i, ..)| *i == buffer)?;
                let side = shadow.slots.resolution;
                (&pack.constants.textures[*slot].view, [side, side])
            } else {
                let colour = targets.buffers[buffer as usize].as_ref()?;
                let side = colour.side(write_alt & (1 << buffer) != 0);
                (&colour.attachment[side], colour.size)
            };
            consistent &= *size.get_or_insert(extent) == extent;
            load_attachment(view)
        })
        .collect();
    if !consistent {
        if state.skipped.insert(format!("{kind:?}")) {
            bevy::log::warn!(
                target: "cinnabar::shaderpack",
                pack = %pack.name, draw = ?kind,
                "draw skipped: its outputs differ in size"
            );
        }
        return;
    }
    let groups = &state.bindings.draws[index].groups;
    let depth = depth.and_then(|(view, _)| depth_attachment(view, false));
    let render = pass(encoder, &format!("{} {kind:?}", pack.name), &attachments, depth);
    let mut tracked = TrackedRenderPass::new(geometry.device, render);
    {
        let raw = tracked.wgpu_pass();
        raw.set_pipeline(&draw.pipeline);
        for (i, group) in groups.iter().enumerate() {
            if let Some(group) = group {
                raw.set_bind_group(i as u32, group, &[]);
            }
        }
    }
    match kind.category() {
        Some(category) => {
            geometry.terrain.draw(&mut tracked, category, kind.shadow());
        }
        None => {
            let (Some(sky), Some(buffer)) = (sky, targets.sky.as_ref()) else {
                return;
            };
            let range = if kind == DrawKind::Sunset {
                sky.sunset.clone()
            } else {
                sky.discs.clone()
            };
            if range.is_empty() {
                return;
            }
            let raw = tracked.wgpu_pass();
            raw.set_vertex_buffer(GEOMETRY_VERTEX_BUFFER, buffer.slice(..));
            raw.set_vertex_buffer(CONSTANT_VERTEX_BUFFER, pack.host.constant_stream.slice(..));
            raw.draw(range, 0..1);
        }
    }
}

/// Copies a depth attachment into float targets with the host's copy pipeline.
fn copy_depth_to_floats(
    device: &Device,
    encoder: &mut CommandEncoder,
    pack: &PackGpu,
    source: &TextureView,
    targets: &[&TextureView],
) {
    if targets.is_empty() {
        return;
    }
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("shader pack depth copy"),
        layout: &pack.host.depth_copy.get_bind_group_layout(0),
        entries: &[wgpu::BindGroupEntry {
            binding: 4,
            resource: wgpu::BindingResource::TextureView(source),
        }],
    });
    for target in targets {
        let mut render = pass(
            encoder,
            "shader pack depth copy",
            &[load_attachment(target)],
            None,
        );
        render.set_pipeline(&pack.host.depth_copy);
        render.set_bind_group(0, &group, &[]);
        render.draw(0..3, 0..1);
    }
}

/// Copies the geometry depth into `depthtex{indices}`: the float copies and, for the ones a
/// program samples with the comparison sampler, the depth copies.
fn copy_view_depth(device: &Device, encoder: &mut CommandEncoder, state: &ViewState, indices: &[usize]) {
    let targets = &state.targets;
    let Some(depth) = targets.geometry_depth.as_ref() else {
        return;
    };
    let floats: Vec<&TextureView> = indices.iter().map(|i| &targets.depth[*i].view).collect();
    copy_depth_to_floats(device, encoder, &state.pack, &depth.view, &floats);
    for index in indices {
        if let Some(compare) = &targets.depth_compare[*index] {
            encoder.copy_texture_to_texture(
                depth.texture.as_image_copy(),
                compare.texture.as_image_copy(),
                depth.texture.size(),
            );
        }
    }
}

/// Copies the shadow depth into `shadowtex{index}` (and `shadowtex0` too when `index` is 1
/// and nothing translucent follows).
fn copy_shadow_depth(device: &Device, encoder: &mut CommandEncoder, pack: &PackGpu, indices: &[usize]) {
    let Some(shadow) = pack.shadow.as_ref() else {
        return;
    };
    let floats: Vec<&TextureView> = indices
        .iter()
        .filter_map(|i| shadow.slots.float[*i])
        .map(|slot| &pack.constants.textures[slot].view)
        .collect();
    copy_depth_to_floats(device, encoder, pack, &shadow.depth_view, &floats);
    for slot in indices.iter().filter_map(|i| shadow.slots.depth[*i]) {
        encoder.copy_texture_to_texture(
            shadow.depth.as_image_copy(),
            pack.constants.textures[slot].texture.as_image_copy(),
            shadow.depth.size(),
        );
    }
}

/// Brings the view's draw bind groups up to date with the atlas.
pub(crate) fn bind_geometry(device: &Device, state: &mut ViewState, geometry: &FrameGeometry<'_>) {
    let generation = geometry.atlas.as_ref().map(|(_, g)| *g);
    if state.atlas_generation == Some(generation) {
        return;
    }
    let pack = std::sync::Arc::clone(&state.pack);
    state.bindings.bind_draws(
        device,
        &pack,
        &state.targets,
        geometry.atlas.as_ref().map(|(views, _)| *views),
    );
    state.atlas_generation = Some(generation);
}

/// The shadow pass: shadow colour clears, opaque shadow casters into `shadowtex1`, then
/// translucent ones into `shadowtex0`, as Iris renders them.
pub(crate) fn record_shadow(
    device: &Device,
    encoder: &mut CommandEncoder,
    state: &mut ViewState,
    geometry: &FrameGeometry<'_>,
) {
    let pack = std::sync::Arc::clone(&state.pack);
    let Some(shadow) = pack.shadow.as_ref() else {
        return;
    };
    for ((_, slot, _), clear) in shadow.slots.color.iter().zip(&shadow.color_clears) {
        if let Some(colour) = clear {
            drop(pass(
                encoder,
                "shader pack shadowcolor clear",
                &[clear_attachment(&pack.constants.textures[*slot].view, *colour)],
                None,
            ));
        }
    }
    clear_depth(encoder, &shadow.depth_view);
    let side = shadow.slots.resolution;
    let depth = Some((&shadow.depth_view, [side, side]));
    for kind in [DrawKind::ShadowSolid, DrawKind::ShadowCutout] {
        record_kind(encoder, state, geometry, kind, 0, depth, None);
    }
    let translucent = pack
        .draws
        .iter()
        .any(|d| d.kind == DrawKind::ShadowTranslucent);
    if translucent {
        copy_shadow_depth(device, encoder, &pack, &[1]);
        record_kind(
            encoder,
            state,
            geometry,
            DrawKind::ShadowTranslucent,
            0,
            depth,
            None,
        );
        copy_shadow_depth(device, encoder, &pack, &[0]);
    } else {
        copy_shadow_depth(device, encoder, &pack, &[0, 1]);
    }
}

/// The sky (no depth), then opaque terrain against a cleared depth attachment, then the
/// copies into `depthtex0..2`, which hold the same depth until translucents draw.
pub(crate) fn record_opaque(
    device: &Device,
    queue: &Queue,
    encoder: &mut CommandEncoder,
    state: &mut ViewState,
    geometry: &FrameGeometry<'_>,
) {
    let scene_alt = state.pack.frame.scene_alt;
    let sky = sky_geometry(geometry.sky);
    if let Some(buffer) = &state.targets.sky {
        queue.write_buffer(buffer, 0, &sky.bytes);
    }
    for kind in [DrawKind::Sky, DrawKind::Sunset] {
        record_kind(encoder, state, geometry, kind, scene_alt, None, Some(&sky));
    }
    let Some(depth) = state.targets.geometry_depth.as_ref() else {
        return;
    };
    // The attachment outlives this function's borrows of `state`; take a view handle.
    let view = depth.view.clone();
    let size = state.targets.screen_size;
    clear_depth(encoder, &view);
    for kind in [DrawKind::TerrainSolid, DrawKind::TerrainCutout] {
        record_kind(
            encoder,
            state,
            geometry,
            kind,
            scene_alt,
            Some((&view, size)),
            None,
        );
    }
    copy_view_depth(device, encoder, state, &[0, 1, 2]);
}

/// Translucent terrain after the deferred passes, then what Vanilla drew in front of the
/// pack's geometry, then `depthtex0` with both.
pub(crate) fn record_translucent(
    device: &Device,
    encoder: &mut CommandEncoder,
    state: &mut ViewState,
    geometry: &FrameGeometry<'_>,
    vanilla: (&TextureView, &TextureView),
) {
    let Some(depth) = state.targets.geometry_depth.as_ref() else {
        return;
    };
    let view = depth.view.clone();
    let size = state.targets.screen_size;
    let translucent_alt = state.pack.frame.translucent_alt;
    record_kind(
        encoder,
        state,
        geometry,
        DrawKind::Translucent,
        translucent_alt,
        Some((&view, size)),
        None,
    );
    copy_view_depth(device, encoder, state, &[0]);
    record_vanilla_overlay(device, encoder, state, vanilla, &view);
    copy_view_depth(device, encoder, state, &[0]);
}

/// Composites the Vanilla scene where it is clearly nearer than the pack's geometry into
/// `colortex0` (gamma-encoded, on the side composite passes read) and the depth attachment.
/// Entities, block entities, particles, clouds and weather reach the pack this way, already
/// lit by Vanilla and without gbuffer data.
fn record_vanilla_overlay(
    device: &Device,
    encoder: &mut CommandEncoder,
    state: &ViewState,
    (colour, depth): (&TextureView, &TextureView),
    attachment: &TextureView,
) {
    let pack = &state.pack;
    let targets = &state.targets;
    let Some(buffer) = targets.buffers[0].as_ref() else {
        return;
    };
    if buffer.size != targets.screen_size {
        return;
    }
    let target = &buffer.attachment[buffer.side(pack.frame.translucent_alt & 1 != 0)];
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("shader pack vanilla overlay"),
        layout: &pack.host.vanilla_overlay.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(colour),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(depth),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: pack.host.depth_parameters.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::TextureView(&targets.depth[0].view),
            },
        ],
    });
    let mut render = pass(
        encoder,
        "shader pack vanilla overlay",
        &[load_attachment(target)],
        depth_attachment(attachment, false),
    );
    render.set_pipeline(&pack.host.vanilla_overlay);
    render.set_bind_group(0, &group, &[]);
    render.draw(0..3, 0..1);
}
