//! Actor bindings recover independently of immutable instance and pose uploads.

use super::*;

#[derive(Default)]
pub(super) struct BindingRetry {
    view: Option<BufferId>,
    retry: crate::actor::gpu::AllocationRetry,
}

impl BindingRetry {
    /// New storage or texture bindings admit an immediate attempt without resetting the retained view.
    pub(super) fn reset(&mut self) {
        self.retry = default();
    }

    /// View-buffer replacement is a new binding identity; persistent pressure otherwise backs off.
    fn ready(&mut self, view: BufferId) -> bool {
        if self.view != Some(view) {
            self.view = Some(view);
            self.reset();
        }
        self.retry.ready()
    }
}

/// Publishes complete bindings or suppresses this draw while preserving retryable uploaded storage.
pub(crate) fn prepare_actor_bind_group(
    render_device: Res<RenderDevice>,
    pipeline_cache: Res<PipelineCache>,
    pipeline: Res<ActorPipeline>,
    view_uniforms: Res<ViewUniforms>,
    mut gpu: ResMut<ActorGpu>,
) {
    // Bindings matter only to draws: without instances they wait instead of being rebuilt every
    // frame, as a rig with no player skins resident (or no actors at all) would otherwise do.
    if gpu.instance_count == 0 {
        return;
    }
    let Some(view_binding) = view_uniforms.uniforms.binding() else {
        gpu.bind_group = None;
        return;
    };
    let view_buffer = view_uniforms
        .uniforms
        .buffer()
        .expect("a dynamic view binding always owns a GPU buffer");
    let view_changed = gpu.view_buffer_id != Some(view_buffer.id());
    if gpu.bind_group.is_some()
        && !view_changed
        && gpu
            .artwork
            .pages
            .iter()
            .flatten()
            .all(|page| page.bind_group.is_some())
    {
        return;
    }
    if !gpu.binding_retry.ready(view_buffer.id()) {
        gpu.instance_count = 0;
        return;
    }
    let Some(geometry_vertex_buffer) = gpu.geometry_vertices.buffer() else {
        gpu.bind_group = None;
        return;
    };
    let Some(geometry_span_buffer) = gpu.geometry_span_buffer.as_ref() else {
        gpu.bind_group = None;
        return;
    };
    let (Some((_, glint_view)), Some((_, item_glint_view))) = (
        gpu.artwork.glint.texture.as_ref(),
        gpu.artwork.item_glint.texture.as_ref(),
    ) else {
        gpu.bind_group = None;
        return;
    };
    let Some((generic_groups, player_group)) =
        gpu_allocation::checked(&render_device, "actor bindings", 0, || {
            let generic_groups: Vec<_> = gpu
                .artwork
                .pages
                .iter()
                .flatten()
                .filter(|page| view_changed || page.bind_group.is_none())
                .map(|page| {
                    render_device.create_bind_group(
                        "neutral actor page bind group",
                        &pipeline_cache.get_bind_group_layout(&pipeline.bind_group_layout),
                        &[
                            BindGroupEntry {
                                binding: 0,
                                resource: view_binding.clone(),
                            },
                            BindGroupEntry {
                                binding: 1,
                                resource: gpu.instance_buffer.as_entire_binding(),
                            },
                            BindGroupEntry {
                                binding: 2,
                                resource: geometry_vertex_buffer.as_entire_binding(),
                            },
                            BindGroupEntry {
                                binding: 3,
                                resource: geometry_span_buffer.as_entire_binding(),
                            },
                            BindGroupEntry {
                                binding: 4,
                                resource: gpu.previous_bone_buffer.as_entire_binding(),
                            },
                            BindGroupEntry {
                                binding: 5,
                                resource: gpu.current_bone_buffer.as_entire_binding(),
                            },
                            BindGroupEntry {
                                binding: 6,
                                resource: BindingResource::TextureView(&page.view),
                            },
                            BindGroupEntry {
                                binding: 7,
                                resource: BindingResource::Sampler(&gpu.sampler),
                            },
                            BindGroupEntry {
                                binding: 8,
                                resource: if page.multitexture {
                                    gpu.multitexture_material.as_entire_binding()
                                } else if page.color_mask {
                                    gpu.color_mask_material.as_entire_binding()
                                } else {
                                    gpu.neutral_material.as_entire_binding()
                                },
                            },
                            BindGroupEntry {
                                binding: 9,
                                resource: BindingResource::TextureView(&gpu.skins.placeholder),
                            },
                            BindGroupEntry {
                                binding: 10,
                                resource: BindingResource::TextureView(&gpu.skins.placeholder),
                            },
                            BindGroupEntry {
                                binding: 12,
                                resource: BindingResource::TextureView(glint_view),
                            },
                            BindGroupEntry {
                                binding: 13,
                                resource: BindingResource::Sampler(&gpu.glint_sampler),
                            },
                            BindGroupEntry {
                                binding: 14,
                                resource: BindingResource::TextureView(item_glint_view),
                            },
                            BindGroupEntry {
                                binding: 11,
                                resource: BindingResource::TextureView(&gpu.skins.placeholder),
                            },
                        ],
                    )
                })
                .collect();
            let player_group = render_device.create_bind_group(
                "instanced standard actor bind group",
                &pipeline_cache.get_bind_group_layout(&pipeline.bind_group_layout),
                &[
                    BindGroupEntry {
                        binding: 0,
                        resource: view_binding,
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: gpu.instance_buffer.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: geometry_vertex_buffer.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 3,
                        resource: geometry_span_buffer.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 4,
                        resource: gpu.previous_bone_buffer.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 5,
                        resource: gpu.current_bone_buffer.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 6,
                        resource: BindingResource::TextureView(gpu.skins.view(0)),
                    },
                    BindGroupEntry {
                        binding: 7,
                        resource: BindingResource::Sampler(&gpu.sampler),
                    },
                    BindGroupEntry {
                        binding: 8,
                        resource: gpu.player_material.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 9,
                        resource: BindingResource::TextureView(gpu.skins.view(1)),
                    },
                    BindGroupEntry {
                        binding: 10,
                        resource: BindingResource::TextureView(gpu.skins.view(2)),
                    },
                    BindGroupEntry {
                        binding: 12,
                        resource: BindingResource::TextureView(glint_view),
                    },
                    BindGroupEntry {
                        binding: 13,
                        resource: BindingResource::Sampler(&gpu.glint_sampler),
                    },
                    BindGroupEntry {
                        binding: 14,
                        resource: BindingResource::TextureView(item_glint_view),
                    },
                    BindGroupEntry {
                        binding: 11,
                        resource: BindingResource::TextureView(gpu.skins.view(3)),
                    },
                ],
            );
            (generic_groups, player_group)
        })
    else {
        gpu.bind_group = None;
        gpu.artwork.invalidate_bindings();
        gpu.binding_retry.retry.failed();
        gpu.instance_count = 0;
        return;
    };
    gpu.bind_group = Some(player_group);
    for (page, group) in gpu
        .artwork
        .pages
        .iter_mut()
        .flatten()
        .filter(|page| view_changed || page.bind_group.is_none())
        .zip(generic_groups)
    {
        page.bind_group = Some(group);
    }
    gpu.view_buffer_id = Some(view_buffer.id());
    gpu.binding_retry.reset();
}
