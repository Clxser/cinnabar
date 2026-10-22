//! One typed recording projection belongs to the current immutable actor-instance publication.

use std::sync::Arc;

use recording_scene::actors::ActorInstance;
use render_scene::actor::ActorRenderInstance;

#[derive(Default)]
pub(super) struct Projection {
    source: Option<Arc<[ActorRenderInstance]>>,
    prepared: Arc<[ActorInstance]>,
}

impl Projection {
    /// Retaining the source prevents pointer reuse; replacement releases the prior publication.
    pub(super) fn project(&mut self, source: &Arc<[ActorRenderInstance]>) -> Arc<[ActorInstance]> {
        if self
            .source
            .as_ref()
            .is_none_or(|old| !Arc::ptr_eq(old, source))
        {
            self.prepared = source.iter().map(project).collect::<Vec<_>>().into();
            self.source = Some(Arc::clone(source));
        }
        Arc::clone(&self.prepared)
    }
}

/// Exhaustive destructuring keeps the recording schema boundary explicit when render fields change.
fn project(instance: &ActorRenderInstance) -> ActorInstance {
    let &ActorRenderInstance {
        runtime_id,
        position,
        pitch_radians,
        yaw_radians,
        head_yaw_radians,
        skin_layer,
    } = instance;
    ActorInstance {
        runtime_id,
        position,
        pitch_radians,
        yaw_radians,
        head_yaw_radians,
        skin_layer,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use render_scene::actor::ActorRenderFrame;

    /// Distinct nonzero fields catch incomplete conversion at the recording boundary.
    fn source() -> Arc<[ActorRenderInstance]> {
        vec![ActorRenderInstance {
            runtime_id: 41,
            position: [1.0, -60.0, 3.0],
            pitch_radians: 0.25,
            yaw_radians: 1.0,
            head_yaw_radians: 0.75,
            skin_layer: 7,
        }]
        .into()
    }

    #[test]
    fn unchanged_instances_reuse_the_projection_without_freezing_other_frame_state() {
        let mut frame = ActorRenderFrame {
            instances: source(),
            instance_revision: 2,
            skin_revision: 3,
            ..Default::default()
        };
        let mut cache = Projection::default();
        let first = super::super::snapshot(&frame, &mut cache);
        frame.instance_revision += 1;
        frame.skin_revision += 1;
        frame.rig.frame_generation = frame.rig.frame_generation.wrapping_add(1);
        let second = super::super::snapshot(&frame, &mut cache);
        assert!(Arc::ptr_eq(&first.instances, &second.instances));
        assert_eq!(
            first.instances.as_ref(),
            &[ActorInstance {
                runtime_id: 41,
                position: [1.0, -60.0, 3.0],
                pitch_radians: 0.25,
                yaw_radians: 1.0,
                head_yaw_radians: 0.75,
                skin_layer: 7,
            }]
        );
        assert_eq!(second.instance_revision, frame.instance_revision);
        assert_eq!(second.skin_revision, frame.skin_revision);
        assert_eq!(second.rig.frame_generation, frame.rig.frame_generation);
        assert_ne!(first.rig.frame_generation, second.rig.frame_generation);
    }

    #[test]
    fn copy_on_write_publications_cannot_mutate_a_queued_sample() {
        let mut source = source();
        let mut cache = Projection::default();
        let first = cache.project(&source);
        Arc::make_mut(&mut source)[0].position[0] = 9.0;
        let second = cache.project(&source);
        assert!(!Arc::ptr_eq(&first, &second));
        assert_eq!(first[0].position[0], 1.0);
        assert_eq!(second[0].position[0], 9.0);
    }

    #[test]
    fn replacement_releases_old_source_and_empty_publication_clears_instances() {
        let mut cache = Projection::default();
        let source = source();
        let weak = Arc::downgrade(&source);
        let first = cache.project(&source);
        drop(source);
        assert!(weak.upgrade().is_some());
        let empty: Arc<[ActorRenderInstance]> = Arc::from([]);
        assert!(cache.project(&empty).is_empty());
        assert!(weak.upgrade().is_none());
        assert_eq!(
            first.len(),
            1,
            "accepted samples keep their own immutable data"
        );
        let prepared = Arc::downgrade(&cache.prepared);
        drop(cache);
        assert!(prepared.upgrade().is_none());
    }

    #[test]
    fn replacement_is_detected_without_relying_on_revision_counters() {
        let mut frame = ActorRenderFrame {
            instances: source(),
            ..Default::default()
        };
        let mut cache = Projection::default();
        let first = super::super::snapshot(&frame, &mut cache);
        let mut changed = source().to_vec();
        changed[0].runtime_id += 1;
        changed[0].skin_layer += 1;
        frame.instances = changed.into();
        let second = super::super::snapshot(&frame, &mut cache);
        assert_eq!(first.instance_revision, second.instance_revision);
        assert_ne!(
            first.instances[0].runtime_id,
            second.instances[0].runtime_id
        );
        assert_ne!(
            first.instances[0].skin_layer,
            second.instances[0].skin_layer
        );
    }
}
