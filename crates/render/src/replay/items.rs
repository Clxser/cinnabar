//! Recorded item slots and terrain-fence identities stay separate from replay GPU generations.

use super::*;

impl ReplayRenderer {
    /// Restores native item data and rebuilds only source-authored rope curves for edited cameras.
    pub(super) fn apply_items(
        &mut self,
        frame: &SceneFrame,
        transform: Transform,
        source_view: bool,
    ) -> Result<(), String> {
        let world = self.app.world_mut();
        let Some(items) = frame.visuals.as_ref().and_then(|v| v.items.as_ref()) else {
            world.insert_resource(render_scene::DroppedItemScene::default());
            return Ok(());
        };
        let previous = world.resource::<render_scene::DroppedItemScene>();
        let revision = if Arc::ptr_eq(&previous.models, &items.models) {
            previous.models_revision
        } else {
            self.generation = self
                .generation
                .checked_add(1)
                .ok_or("Replay item generation overflowed.")?;
            self.generation
        };
        let dynamic = if source_view {
            Arc::clone(&items.dynamic)
        } else {
            let mut lines = Vec::new();
            for rope in items.ropes.iter() {
                rope.append(transform.translation.to_array(), &mut lines);
            }
            lines.into()
        };
        world.insert_resource(render_scene::DroppedItemScene {
            models_revision: revision,
            models: Arc::clone(&items.models),
            instances: Arc::clone(&items.instances),
            terrain_session_id: items.terrain_session_id,
            terrain_instances: items
                .terrain_instances
                .iter()
                .map(|candidate| {
                    let mut visible = candidate.visible;
                    if let Some(terrain) = &items.terrain {
                        for transition in candidate.transitions.iter() {
                            if terrain
                                .generations
                                .binary_search_by_key(&transition.key, |(key, _)| *key)
                                .is_ok_and(|index| {
                                    terrain.generations[index].1 >= transition.generation
                                })
                            {
                                visible = transition.visible;
                            }
                        }
                    }
                    render_model::TerrainItemInstance {
                        instance: candidate.instance,
                        visible,
                        transitions: Arc::from([]),
                    }
                })
                .collect::<Vec<_>>()
                .into(),
            dynamic,
            daylight: items.daylight,
            glint: items.glint.clone(),
            glint_scroll: items.glint_scroll,
        });
        Ok(())
    }
}
