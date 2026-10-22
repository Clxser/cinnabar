//! Recording-only item model admission shares native geometry without advancing live caches.

use super::*;
use bevy::prelude::Resource;
use render_model::dropped_item::{DroppedItemSpawnPose, PreparedItemRope};

#[derive(Clone, Copy, Debug, Default)]
pub struct Admission {
    pub model_failures: u32,
    pub instance_limit: bool,
    pub rope_limit: bool,
}

#[derive(Resource, Default)]
pub struct ReplayItems {
    enabled: bool,
    requested: bool,
    cache: ModelCache,
    scratch: PublishScratch,
    source_poses: HashMap<(u64, u64), DroppedItemSpawnPose>,
    pub scene: Option<DroppedItemScene>,
    pub ropes: Arc<[PreparedItemRope]>,
    pub admission: Admission,
    pub source_seconds: f64,
}

impl ReplayItems {
    /// Owns the recording-only item cache and releases every retained epoch when capture ends.
    pub fn set_enabled(&mut self, enabled: bool) {
        if self.enabled && !enabled {
            *self = Self::default();
        }
        self.enabled = enabled;
    }

    /// The capture owner admits one independent received-item preparation at video cadence.
    pub fn request_frame(&mut self) {
        self.requested = self.enabled;
    }

    /// Copies bounded lifetime inputs after their actual first native render admission.
    pub(super) fn remember_source_poses(
        &mut self,
        poses: &HashMap<(u64, u64), DroppedItemSpawnPose>,
        stream: &WorldStream,
        partial_tick: f32,
    ) {
        if !self.enabled || !self.requested {
            return;
        }
        self.source_poses.clear();
        stream
            .authority()
            .dropped_items_into(partial_tick, &mut self.scratch.dropped);
        for view in self
            .scratch
            .dropped
            .iter()
            .take(render_model::MAX_DROPPED_ITEM_INSTANCES)
        {
            let key = (view.runtime_id, view.spawn_revision);
            if let Some(pose) = poses.get(&key) {
                self.source_poses.insert(key, *pose);
            }
        }
        self.scratch.dropped.clear();
    }

    /// Prepares actual received item actors and independent block contents with no source frustum.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        icons: &UiPresentationRuntime,
        stream: &WorldStream,
        collisions: Option<&dyn crate::observations::CollisionLookup>,
        camera: ([f32; 3], f32),
        partial_tick: f32,
        source_seconds: f64,
        textures: Option<&ChunkTextureAssets>,
        placements: &[render_model::StaticItemPlacement],
        device: Option<&bevy::render::renderer::RenderDevice>,
        source: &DroppedItemScene,
    ) {
        if !self.enabled || !std::mem::take(&mut self.requested) {
            return;
        }
        self.cache.sync(
            stream.authority().actor_session_id(),
            textures.map(|v| v.identity()),
            icons.session_icon_generation(),
        );
        self.cache
            .spawn_poses
            .extend(self.source_poses.iter().map(|(key, pose)| (*key, *pose)));
        let mut scene = self.scene.take().unwrap_or_default();
        self.admission = DroppedItemPublisher::prepare_scene(
            &mut scene,
            &mut self.cache,
            &mut self.scratch,
            textures,
            placements,
            device,
            icons,
            stream,
            collisions,
            Some(camera),
            None,
            partial_tick,
            (None, render_model::ActorGlint::default()),
            true,
        );
        scene.set_glint(source.glint.clone(), source.glint_scroll);
        let (ropes, limited) = ropes(stream, partial_tick);
        self.ropes = ropes;
        self.admission.rope_limit |= limited;
        self.source_seconds = source_seconds;
        self.scene = Some(scene);
    }
}

/// Resolves the owner's authored native tessellation inputs, bounding the retained prefix by vertex capacity.
pub(super) fn ropes(stream: &WorldStream, partial_tick: f32) -> (Arc<[PreparedItemRope]>, bool) {
    let mut result = Vec::new();
    let mut vertices = 0usize;
    let mut limited = false;
    for rope in stream.authority().ropes(partial_tick) {
        if vertices >= render_model::MAX_DYNAMIC_ITEM_VERTICES {
            limited = true;
            break;
        }
        let rope = prepare_rope(rope);
        vertices += rope.segments as usize * 6;
        result.push(rope);
    }
    (
        result.into(),
        limited || vertices > render_model::MAX_DYNAMIC_ITEM_VERTICES,
    )
}

/// One native rope kind selects its tessellation policy in this single owner.
pub(super) fn prepare_rope(rope: client_world::RopeView) -> PreparedItemRope {
    let length = rope
        .from
        .iter()
        .zip(&rope.to)
        .map(|(a, b)| (a - b) * (a - b))
        .sum::<f32>()
        .sqrt();
    let (segments, sag, half_width, color) = match rope.kind {
        RopeKind::FishingLine => (
            FISHING_SEGMENTS,
            length * FISHING_SAG_FRACTION,
            FISHING_HALF_WIDTH,
            rope_color(0, 0, 0),
        ),
        RopeKind::Lead => (
            LEAD_SEGMENTS,
            length * LEAD_SAG_FRACTION,
            LEAD_HALF_WIDTH,
            rope_color(127, 96, 55),
        ),
    };
    PreparedItemRope {
        from: rope.from,
        to: rope.to,
        segments: segments as u32,
        sag,
        half_width,
        color,
    }
}
