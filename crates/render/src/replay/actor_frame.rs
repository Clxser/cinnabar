//! Camera-only edits preserve the immutable actor publication and its GPU generation.

use std::sync::Arc;

use recording_scene::actors::{ActorInstance, ActorSample, FrozenActors};
use render_model::ActorRigRenderFrame;
use render_scene::{ActorRenderFrame, ActorRenderInstance};

use super::visibility::ActorVisibility;

#[derive(Default)]
pub(super) struct Publication {
    source: Option<FrozenActors>,
    visibility: Option<ActorVisibility>,
}

impl Publication {
    /// Retains one source root to prevent address reuse; seeks invalidate even identical source data.
    pub(super) fn prepare(
        &mut self,
        source: &FrozenActors,
        visibility: ActorVisibility,
        reset: bool,
        generation: &mut u64,
    ) -> Result<Option<ActorRenderFrame>, String> {
        if !reset
            && self.visibility.as_ref() == Some(&visibility)
            && self
                .source
                .as_ref()
                .is_some_and(|old| same_source(old, source))
        {
            return Ok(None);
        }
        let next = generation
            .checked_add(1)
            .ok_or("Replay actor generation overflowed.")?;
        let frame = project(source, &visibility, next);
        self.source = Some(source.clone());
        self.visibility = Some(visibility);
        *generation = next;
        Ok(Some(frame))
    }
}

/// Every immutable source allocation and scalar used by the published actor frame participates.
fn same_source(a: &FrozenActors, b: &FrozenActors) -> bool {
    let FrozenActors {
        sample,
        skins,
        artwork,
        geometry_identity,
        appearance_identity,
    } = a;
    let ActorSample {
        coverage,
        instances,
        instance_revision,
        skin_revision,
        rig,
        instance_pages,
    } = sample;
    *geometry_identity == b.geometry_identity
        && *appearance_identity == b.appearance_identity
        && *coverage == b.sample.coverage
        && *instance_revision == b.sample.instance_revision
        && *skin_revision == b.sample.skin_revision
        && Arc::ptr_eq(instances, &b.sample.instances)
        && Arc::ptr_eq(instance_pages, &b.sample.instance_pages)
        && Arc::ptr_eq(skins, &b.skins)
        && Arc::ptr_eq(artwork, &b.artwork)
        && same_rig(rig, &b.sample.rig)
}

/// Captured generations alone cannot identify independently interpolated pose buffers.
pub(super) fn same_rig(a: &ActorRigRenderFrame, b: &ActorRigRenderFrame) -> bool {
    let ActorRigRenderFrame {
        frame_generation,
        geometry_revision,
        instances,
        previous_bones,
        current_bones,
        geometry_vertices,
        geometry_spans,
        manifest,
        maximum_vertex_count,
        rejects,
    } = a;
    *frame_generation == b.frame_generation
        && *geometry_revision == b.geometry_revision
        && *maximum_vertex_count == b.maximum_vertex_count
        && *rejects == b.rejects
        && geometry_vertices.epoch == b.geometry_vertices.epoch
        && geometry_vertices.len() == b.geometry_vertices.len()
        && Arc::ptr_eq(&geometry_vertices.segments, &b.geometry_vertices.segments)
        && Arc::ptr_eq(&geometry_vertices.offsets, &b.geometry_vertices.offsets)
        && Arc::ptr_eq(geometry_spans, &b.geometry_spans)
        && Arc::ptr_eq(instances, &b.instances)
        && Arc::ptr_eq(previous_bones, &b.previous_bones)
        && Arc::ptr_eq(current_bones, &b.current_bones)
        && Arc::ptr_eq(manifest, &b.manifest)
}

/// Unmasked rig arrays remain shared; masked draws retain source bone bases and acquire packed indices.
fn project(
    source: &FrozenActors,
    visibility: &ActorVisibility,
    generation: u64,
) -> ActorRenderFrame {
    let sample = &source.sample;
    let mut rig = sample.rig.clone();
    let keep = |index: usize| visibility.visible(&rig.manifest[index].identity);
    let masked = (0..rig.instances.len()).any(|index| !keep(index));
    let (instances, instance_pages) = if masked {
        let selected = (0..rig.instances.len())
            .filter(|&index| keep(index))
            .collect::<Vec<_>>();
        let instances = selected
            .iter()
            .map(|&index| instance(&sample.instances[index]))
            .collect::<Vec<_>>()
            .into();
        let pages = selected
            .iter()
            .map(|&index| sample.instance_pages[index])
            .collect::<Vec<_>>()
            .into();
        rig.instances = selected
            .iter()
            .map(|&index| rig.instances[index])
            .collect::<Vec<_>>()
            .into();
        rig.manifest = selected
            .iter()
            .enumerate()
            .map(|(next, &index)| {
                let mut entry = rig.manifest[index].clone();
                entry.instance_index = next as u32;
                entry
            })
            .collect::<Vec<_>>()
            .into();
        (instances, pages)
    } else {
        (
            sample
                .instances
                .iter()
                .map(instance)
                .collect::<Vec<_>>()
                .into(),
            Arc::clone(&sample.instance_pages),
        )
    };
    rig.frame_generation = generation;
    ActorRenderFrame {
        mask_only_runtime_ids: Arc::from([]),
        instances,
        skins: Arc::clone(&source.skins),
        instance_revision: sample.instance_revision,
        skin_revision: sample.skin_revision,
        rig,
        artwork: Arc::clone(&source.artwork),
        instance_pages,
    }
}

/// Typed recording and renderer instances share a contract, with exhaustive field conversion.
fn instance(source: &ActorInstance) -> ActorRenderInstance {
    let &ActorInstance {
        runtime_id,
        position,
        pitch_radians,
        yaw_radians,
        head_yaw_radians,
        skin_layer,
    } = source;
    ActorRenderInstance {
        runtime_id,
        position,
        pitch_radians,
        yaw_radians,
        head_yaw_radians,
        skin_layer,
    }
}

#[cfg(test)]
mod tests;
