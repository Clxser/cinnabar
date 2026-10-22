//! Source hiding and editor camera intersections retain separate lifetime and layer policies.

use recording_scene::targets::TargetFrame;
use recording_studio::camera::CameraTargetRef;
use render_model::ActorRenderIdentity;

mod intersection;
mod volume;
pub(super) use intersection::IntersectionCache;

/// Geometry masking names one draw layer of one spawn, never every actor sharing a runtime ID.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct DrawKey {
    session: u64,
    dimension: i32,
    runtime: u64,
    spawn: u64,
    layer: u8,
}

impl From<&ActorRenderIdentity> for DrawKey {
    /// Camera motion does not change which received spawn and layer owns these triangles.
    fn from(identity: &ActorRenderIdentity) -> Self {
        Self {
            session: identity.session_id,
            dimension: identity.dimension,
            runtime: identity.runtime_id,
            spawn: identity.spawn_revision,
            layer: identity.layer,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ActorVisibility {
    local: u64,
    recorded_hide_local: bool,
    spectated: Option<CameraTargetRef>,
    intersections: Vec<DrawKey>,
}

impl ActorVisibility {
    /// Recorded first-person and spectated views preserve their existing complete-actor policies.
    pub(super) fn new(
        local_actor: u64,
        source_body_visible: Option<bool>,
        targets: Option<&TargetFrame>,
        recorded_view: bool,
        spectated: Option<CameraTargetRef>,
    ) -> Self {
        let spectated = spectated.and_then(|reference| {
            let target = targets?.target(reference.runtime_id)?;
            target.matches(reference).then(|| target.reference())
        });
        Self {
            local: local_actor,
            recorded_hide_local: recorded_view && source_body_visible == Some(false),
            spectated,
            intersections: Vec::new(),
        }
    }

    /// Admits the sorted, deduplicated layers produced by this renderer's intersection query.
    pub(super) fn with_intersections(mut self, intersections: Vec<DrawKey>) -> Self {
        self.intersections = intersections;
        self
    }

    /// Source policy may hide a complete spawn; geometric masking preserves its other layers.
    pub(super) fn visible(&self, identity: &ActorRenderIdentity) -> bool {
        !(self.recorded_hide_local && identity.runtime_id == self.local)
            && !self.spectated.is_some_and(|target| {
                target.runtime_id == identity.runtime_id
                    && target
                        .lifetime
                        .is_some_and(|lifetime| lifetime.spawn_revision == identity.spawn_revision)
            })
            && self
                .intersections
                .binary_search(&DrawKey::from(identity))
                .is_err()
    }
}

#[cfg(test)]
mod tests;
