//! Versioned metadata keeps identities exact without serializing native enum discriminants.

use render_api::PoseGeneration;
use render_model::{
    ActorDrawManifestEntry, ActorRenderIdentity, ActorRigRejects, ActorRigRoute, EntityRigId,
};
use serde::{Deserialize, Serialize};

use super::{ActorCoverage, ActorInstance, ActorSample};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Metadata {
    pub coverage: ActorCoverage,
    pub instances: Vec<ActorInstance>,
    pub instance_revision: u64,
    pub skin_revision: u64,
    pub frame_generation: u64,
    pub geometry_revision: u64,
    pub maximum_vertex_count: u32,
    pub manifest: Vec<Manifest>,
    pub rejects: Rejects,
}

impl Metadata {
    /// Exhaustive native frame destructuring forces a review when its contract grows.
    pub fn capture(sample: &ActorSample) -> Self {
        let render_model::ActorRigRenderFrame {
            frame_generation,
            geometry_revision,
            instances: _,
            previous_bones: _,
            current_bones: _,
            geometry_vertices: _,
            geometry_spans: _,
            manifest,
            maximum_vertex_count,
            rejects,
        } = &sample.rig;
        Self {
            coverage: sample.coverage,
            instances: sample.instances.to_vec(),
            instance_revision: sample.instance_revision,
            skin_revision: sample.skin_revision,
            frame_generation: *frame_generation,
            geometry_revision: *geometry_revision,
            maximum_vertex_count: *maximum_vertex_count,
            manifest: manifest.iter().map(Manifest::capture).collect(),
            rejects: Rejects::capture(*rejects),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    session_id: u64,
    dimension: i32,
    runtime_id: u64,
    spawn_revision: u64,
    ingress_sequence: u64,
    source_tick: Option<u64>,
    movement_revision: u64,
    pose_generation: u64,
    layer: u8,
}
impl Identity {
    /// Source session and pose generations remain evidence, never authority for a live session.
    fn capture(value: ActorRenderIdentity) -> Self {
        let ActorRenderIdentity {
            session_id,
            dimension,
            runtime_id,
            spawn_revision,
            ingress_sequence,
            source_tick,
            movement_revision,
            pose_generation,
            layer,
        } = value;
        Self {
            session_id,
            dimension,
            runtime_id,
            spawn_revision,
            ingress_sequence,
            source_tick,
            movement_revision,
            pose_generation: pose_generation.get(),
            layer,
        }
    }

    /// Restores the owner's typed generation before identity admission.
    fn restore(self) -> ActorRenderIdentity {
        let Self {
            session_id,
            dimension,
            runtime_id,
            spawn_revision,
            ingress_sequence,
            source_tick,
            movement_revision,
            pose_generation,
            layer,
        } = self;
        ActorRenderIdentity {
            session_id,
            dimension,
            runtime_id,
            spawn_revision,
            ingress_sequence,
            source_tick,
            movement_revision,
            pose_generation: PoseGeneration::new(pose_generation),
            layer,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Route {
    Compiled,
    StaticFallback,
    Diagnostic,
    NoDraw,
}
impl Route {
    /// Exhaustive matches keep the on-disk vocabulary independent of native enum layout.
    fn capture(value: ActorRigRoute) -> Self {
        match value {
            ActorRigRoute::Compiled => Self::Compiled,
            ActorRigRoute::StaticFallback => Self::StaticFallback,
            ActorRigRoute::Diagnostic => Self::Diagnostic,
            ActorRigRoute::NoDraw => Self::NoDraw,
        }
    }
    /// Restores only variants supported by this schema version.
    fn restore(self) -> ActorRigRoute {
        match self {
            Self::Compiled => ActorRigRoute::Compiled,
            Self::StaticFallback => ActorRigRoute::StaticFallback,
            Self::Diagnostic => ActorRigRoute::Diagnostic,
            Self::NoDraw => ActorRigRoute::NoDraw,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Manifest {
    identity: Identity,
    rig: u32,
    completed_tick: u64,
    reset_generation: u64,
    route: Route,
    instance_index: u32,
    previous_bone_base: u32,
    current_bone_base: u32,
    bone_count: u32,
}
impl Manifest {
    /// Captures every native draw association alongside its packed GPU instance.
    fn capture(value: &ActorDrawManifestEntry) -> Self {
        let &ActorDrawManifestEntry {
            identity,
            rig,
            completed_tick,
            reset_generation,
            route,
            instance_index,
            previous_bone_base,
            current_bone_base,
            bone_count,
        } = value;
        Self {
            identity: Identity::capture(identity),
            rig: rig.0,
            completed_tick,
            reset_generation,
            route: Route::capture(route),
            instance_index,
            previous_bone_base,
            current_bone_base,
            bone_count,
        }
    }
    /// Cross-references are checked against admitted geometry and bone arenas by the pose decoder.
    pub fn restore(self) -> ActorDrawManifestEntry {
        let Self {
            identity,
            rig,
            completed_tick,
            reset_generation,
            route,
            instance_index,
            previous_bone_base,
            current_bone_base,
            bone_count,
        } = self;
        ActorDrawManifestEntry {
            identity: identity.restore(),
            rig: EntityRigId(rig),
            completed_tick,
            reset_generation,
            route: route.restore(),
            instance_index,
            previous_bone_base,
            current_bone_base,
            bone_count,
        }
    }
}

// One schema field list generates both exhaustive conversions, so added native counters fail to compile.
macro_rules! rejects {
    ($($field:ident),+ $(,)?) => {
        #[derive(Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub(super) struct Rejects { $($field: u64),+ }
        impl Rejects {
            /// Retains rejection diagnostics without substituting silently missing draws.
            fn capture(value: ActorRigRejects) -> Self {
                let ActorRigRejects { $($field),+ } = value; Self { $($field),+ }
            }
            /// Native construction is exhaustive and therefore checked when counters change.
            pub fn restore(self) -> ActorRigRejects {
                let Self { $($field),+ } = self; ActorRigRejects { $($field),+ }
            }
        }
    };
}
rejects!(
    invalid_identity,
    invalid_world_transform,
    non_finite_pose,
    pose_length_mismatch,
    bone_capacity,
    actor_capacity,
    missing_geometry,
    invalid_geometry,
    no_draw,
    generation_exhaustion
);
