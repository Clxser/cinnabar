use crate::chunk::*;
use render_scene::{ChunkRenderInstance, ChunkTextureAssets};
use std::sync::Weak;

/// Retains validation results only while the same immutable upload is waiting.
#[derive(Default)]
pub(in crate::chunk) struct ModelValidationCache {
    entries: HashMap<Entity, ValidatedModelStreams>,
}

struct ValidatedModelStreams {
    generation: u64,
    assets: Weak<RuntimeAssets>,
    asset_revision: u64,
    refs: Weak<[PackedModelRef]>,
    lighting: Weak<[PackedQuadLighting]>,
    opaque: Weak<[PackedModelDrawRef]>,
    blend: Weak<[PackedModelDrawRef]>,
    valid: bool,
}

impl ValidatedModelStreams {
    /// Allows weak identities only while other owners still retain every input allocation.
    fn has_owners(&self) -> bool {
        self.assets.strong_count() > 0
            && self.refs.strong_count() > 0
            && self.lighting.strong_count() > 0
            && self.opaque.strong_count() > 0
            && self.blend.strong_count() > 0
    }
}

impl ModelValidationCache {
    /// Returns validity and whether validation ran, invalidating on any stream or asset change.
    pub(in crate::chunk) fn validate(
        &mut self,
        entity: Entity,
        instance: &ChunkRenderInstance,
        assets: &ChunkTextureAssets,
        limit: usize,
    ) -> (bool, bool) {
        if let Some(entry) = self.entries.get(&entity)
            && entry.generation == instance.generation
            && entry.asset_revision == assets.revision
            && same_allocation(&entry.assets, assets.assets())
            && same_allocation(&entry.refs, &instance.model_refs)
            && same_allocation(&entry.lighting, &instance.model_lighting)
            && same_allocation(&entry.opaque, &instance.model_draw_refs)
            && same_allocation(&entry.blend, &instance.transparent_model_draw_refs)
        {
            return (entry.valid, false);
        }
        let valid = meshing::model_validation::validate_partitioned_model_streams(
            &instance.model_refs,
            &instance.model_lighting,
            &instance.model_draw_refs,
            &instance.transparent_model_draw_refs,
            assets.assets().model_templates(),
            assets.assets().model_quads(),
            assets.assets().materials(),
        );
        if self.entries.len() < limit || self.entries.contains_key(&entity) {
            self.entries.insert(
                entity,
                ValidatedModelStreams {
                    generation: instance.generation,
                    assets: Arc::downgrade(assets.assets()),
                    asset_revision: assets.revision,
                    refs: Arc::downgrade(&instance.model_refs),
                    lighting: Arc::downgrade(&instance.model_lighting),
                    opaque: Arc::downgrade(&instance.model_draw_refs),
                    blend: Arc::downgrade(&instance.transparent_model_draw_refs),
                    valid,
                },
            );
        }
        (valid, true)
    }

    /// Drops completed entries and releases obsolete allocations even while uploads are blocked.
    pub(in crate::chunk) fn retain_waiters(&mut self, waiters: &HashMap<Entity, u32>) {
        self.entries
            .retain(|entity, entry| waiters.contains_key(entity) && entry.has_owners());
    }
}

/// Compares allocation identities while weak references prevent address reuse.
fn same_allocation<T: ?Sized>(previous: &Weak<T>, current: &Arc<T>) -> bool {
    std::ptr::eq(previous.as_ptr(), Arc::as_ptr(current))
}
