//! Render-space endpoints remain owned by the sampled frame which produced their immutable layers.

use super::*;
use render_model::RenderBoneTransform;

/// A sampled native body and persona layers, shared by live and received-world consumers.
#[derive(Clone)]
pub struct ActorBodySample {
    pub bones: [Arc<[RenderBoneTransform]>; 2],
    pub skin: Option<Arc<[SkinRenderLayer]>>,
}

/// A different skeleton can select another layer; retain at most one conversion for this actor frame.
#[derive(Default)]
pub(super) struct Cache {
    entry: Option<(usize, Option<ActorBodySample>)>,
    #[cfg(test)]
    endpoints: usize,
}

impl Cache {
    /// Borrowed tick layers use the tick-owned conversion; only sampled native body layers convert here.
    pub(super) fn sample(
        &mut self,
        layers: &ActorRenderLayers<'_>,
        lengths: [usize; 2],
    ) -> Option<ActorBodySample> {
        let Cow::Owned(render) = &layers.render else {
            return None;
        };
        let index = render.iter().position(|layer| {
            layer.geometry.is_none()
                && !layer.pose.is_empty()
                && layer.previous_pose.len() == lengths[0]
                && layer.pose.len() == lengths[1]
        })?;
        if self.entry.as_ref().is_none_or(|(old, _)| *old != index) {
            let sample = self.convert(layers, &render[index]);
            self.entry = Some((index, sample));
        }
        self.entry.as_ref()?.1.clone()
    }

    /// Equal native endpoints share one render allocation; invalid bones never publish a partial sample.
    fn convert(
        &mut self,
        layers: &ActorRenderLayers<'_>,
        layer: &RenderTextureLayer,
    ) -> Option<ActorBodySample> {
        let current = self.endpoint(&layer.pose)?;
        let previous = if Arc::ptr_eq(&layer.previous_pose, &layer.pose) {
            Arc::clone(&current)
        } else {
            self.endpoint(&layer.previous_pose)?
        };
        let skin = match &layers.skin {
            Cow::Owned(skin) => Some(Arc::from(skin.as_slice())),
            Cow::Borrowed(_) => None,
        };
        Some(ActorBodySample {
            bones: [previous, current],
            skin,
        })
    }

    /// One exact-size shared allocation preserves the existing model-space finite-value admission.
    fn endpoint(&mut self, bones: &[BoneTransform]) -> Option<Arc<[RenderBoneTransform]>> {
        #[cfg(test)]
        {
            self.endpoints += 1;
        }
        let mut finite = true;
        let converted: Arc<[RenderBoneTransform]> = bones
            .iter()
            .map(|bone| {
                RenderBoneTransform::from_model_space_scaled(
                    bone.rotation,
                    bone.translation_scale,
                    bone.axis_scale,
                )
                .unwrap_or_else(|| {
                    finite = false;
                    RenderBoneTransform::default()
                })
            })
            .collect();
        finite.then_some(converted)
    }
}

#[cfg(test)]
mod tests;
