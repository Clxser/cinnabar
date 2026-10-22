//! Native particle facing and sorting are evaluated from recorded received candidates.

use super::ReplayRenderer;
use bevy::prelude::*;
use recording_scene::playback::SceneFrame;

impl ReplayRenderer {
    /// The source view keeps its exact prepared cave selection; edited views use all received samples.
    pub(super) fn reproject_particles(
        &mut self,
        frame: &SceneFrame,
        transform: Transform,
        fov_degrees: f64,
        source_view: bool,
    ) {
        if source_view {
            return;
        }
        let Some(received) = frame
            .visuals
            .as_ref()
            .and_then(|visuals| visuals.particles.received.as_deref())
        else {
            return;
        };
        let half_height = (fov_degrees as f32 * std::f32::consts::PI / 360.0).tan();
        let half_width = half_height * self.size[0] as f32 / self.size[1] as f32;
        let view = particles::ParticleView {
            position: transform.translation.to_array(),
            right: transform.right().to_array(),
            up: transform.up().to_array(),
            forward: transform.forward().to_array(),
            half_diagonal: half_height.hypot(half_width).atan(),
        };
        let lists = particles::replay::draw(received, &view);
        let mut output = self
            .app
            .world_mut()
            .resource_mut::<render_scene::ParticleGpuFrame>();
        output.opaque = lists.opaque.into();
        output.blend = lists.blend.into();
        output.add = lists.add.into();
        output.cave_hidden = 0;
    }
}
