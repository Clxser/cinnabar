//! Binds the sorted transparent pass's hidden-slot mask; see `gpu_cull::transparent`.
//!
//! Non-Enhanced transparent pipelines read the mask at group 2. A draw binds the mask only when
//! the late pass rewrote it for the draw's own view this frame; every other draw, including every
//! draw on devices without the GPU cull, binds a mask that hides nothing.

use crate::chunk::gpu_cull::GpuCull;
use crate::chunk::*;

/// Group 2 of the non-Enhanced transparent terrain pipeline.
pub(in crate::chunk) fn layout() -> BindGroupLayoutDescriptor {
    BindGroupLayoutDescriptor::new(
        "transparent terrain hidden slots layout",
        &[BindGroupLayoutEntry {
            binding: 0,
            visibility: ShaderStages::VERTEX,
            ty: BindingType::Texture {
                sample_type: TextureSampleType::Uint,
                view_dimension: TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        }],
    )
}

/// The always-visible mask and the bindings of the current one.
#[derive(Resource, Default)]
pub(in crate::chunk) struct TransparentOcclusionBindings {
    visible: Option<BindGroup>,
    hidden: Option<(wgpu::TextureView, BindGroup)>,
}

impl TransparentOcclusionBindings {
    /// The group a transparent draw in `view` binds: the current mask only when the late pass
    /// rewrote it for `encoded`, that same view.
    fn group(&self, view: Entity, encoded: Option<Entity>) -> Option<&BindGroup> {
        match &self.hidden {
            Some((_, hidden)) if encoded == Some(view) => Some(hidden),
            _ => self.visible.as_ref(),
        }
    }
}

/// Binds the current or all-visible mask, after the GPU cull prepared this frame's view.
pub(in crate::chunk) fn prepare_transparent_occlusion(
    mut bindings: ResMut<TransparentOcclusionBindings>,
    cull: Option<Res<GpuCull>>,
    pipeline_cache: Res<PipelineCache>,
    device: Res<RenderDevice>,
) {
    let bindings = &mut *bindings;
    let layout = pipeline_cache.get_bind_group_layout(&layout());
    let group = |view: &wgpu::TextureView| {
        device.create_bind_group(
            "transparent terrain hidden slots",
            &layout,
            &[BindGroupEntry {
                binding: 0,
                resource: BindingResource::TextureView(view),
            }],
        )
    };
    if bindings.visible.is_none() {
        // Textures start zeroed, so this one hides nothing.
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("transparent terrain visible slots"),
            size: Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: crate::chunk::gpu_cull::kernels::TRANSPARENT_MASK_FORMAT,
            usage: TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        bindings.visible = Some(group(&texture.create_view(&Default::default())));
    }
    let mask = cull.as_deref().and_then(|cull| cull.transparent.mask());
    match mask {
        Some(mask)
            if bindings
                .hidden
                .as_ref()
                .is_none_or(|(view, _)| *view != mask.view) =>
        {
            bindings.hidden = Some((mask.view.clone(), group(&mask.view)));
        }
        Some(_) => {}
        None => bindings.hidden = None,
    }
}

/// Binds group `I` of the non-Enhanced transparent pipeline; Enhanced views use that slot for
/// their own group and draw without the mask.
pub(in crate::chunk) struct SetTransparentOcclusion<const I: usize>;

impl<P: PhaseItem, const I: usize> RenderCommand<P> for SetTransparentOcclusion<I> {
    type Param = (
        Option<SRes<TransparentOcclusionBindings>>,
        Option<SRes<GpuCull>>,
    );
    type ViewQuery = (Entity, Has<render_scene::EnhancedRendering>);
    type ItemQuery = ();

    fn render<'w>(
        _item: &P,
        (view, enhanced): ROQueryItem<'w, '_, Self::ViewQuery>,
        _entity: Option<ROQueryItem<'w, '_, Self::ItemQuery>>,
        (bindings, cull): SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        if crate::enhanced::view_group_active(enhanced) {
            return RenderCommandResult::Success;
        }
        let encoded = cull.and_then(|cull| cull.into_inner().transparent.encoded_view());
        let Some(group) = bindings.and_then(|bindings| bindings.into_inner().group(view, encoded))
        else {
            return RenderCommandResult::Skip;
        };
        pass.set_bind_group(I, group, &[]);
        RenderCommandResult::Success
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_rewritten_view_binds_the_current_mask() {
        let (device, _) = wgpu::Device::noop(&Default::default());
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[],
        });
        let group = || {
            BindGroup::from(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &layout,
                entries: &[],
            }))
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d::default(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let (culled, other) = (
            Entity::from_raw_u32(7).unwrap(),
            Entity::from_raw_u32(8).unwrap(),
        );
        let bindings = TransparentOcclusionBindings {
            visible: Some(group()),
            hidden: Some((texture.create_view(&Default::default()), group())),
        };
        let hidden = bindings.hidden.as_ref().map(|(_, group)| group.id());
        let visible = bindings.visible.as_ref().map(BindGroup::id);
        let bound = |view, encoded| bindings.group(view, encoded).map(BindGroup::id);
        assert_eq!(bound(culled, Some(culled)), hidden);
        assert_eq!(bound(other, Some(culled)), visible);
        assert_eq!(
            bound(culled, None),
            visible,
            "a frame without a rewrite must not read the last mask"
        );
    }
}
