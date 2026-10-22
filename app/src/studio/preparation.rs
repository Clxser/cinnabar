//! One capture-clock admission controls all extra received-world preparation.

use super::*;

#[derive(Resource, Default)]
pub(super) struct Cadence(Option<(bool, u64)>);

/// Requests work before gameplay preparation, using the same timestamp as the journal sampler.
pub(super) fn request(world: &mut World) {
    let capture = world.resource::<NativeCapture>();
    let enabled = capture.armed || capture.startup.is_some() || capture.active.is_some();
    let now = world.resource::<Time<Real>>().elapsed_secs_f64();
    let active = capture.active.as_ref();
    let settings = active.map_or(capture.settings, |active| active.session.settings());
    let elapsed = active.map_or(now, |active| (now - active.started).max(0.0));
    let key = enabled.then(|| {
        let frame = MediaTime::from_seconds(elapsed)
            .ok()
            .and_then(|at| settings.fps.frame_at(at).ok())
            .unwrap_or(0);
        (active.is_some(), frame)
    });
    let mut cadence = world.resource_mut::<Cadence>();
    let due = key.is_some() && cadence.0 != key;
    cadence.0 = key;
    if let Some(mut presentation) =
        world.get_resource_mut::<client_shell::ui_resources::UiPresentationResource>()
    {
        presentation.request_nametag_recording(enabled, due);
    }
    let mut actors =
        world.resource_mut::<client_presentation::actor_publication::recording::ReplayActors>();
    actors.set_enabled(enabled);
    if due {
        actors.request_frame();
    }
    if let Some(mut particles) = world.get_resource_mut::<render_scene::ParticleSimulation>() {
        particles.set_recording(enabled);
        if due {
            particles.request_recording_frame();
        }
    }
    if let Some(mut blocks) =
        world.get_resource_mut::<crate::block_entities::system::recording::ReplayBlockEntities>()
    {
        blocks.set_enabled(enabled);
        if due {
            blocks.request_frame();
        }
    }
    let mut items =
        world.resource_mut::<client_presentation::dropped_items::recording::ReplayItems>();
    items.set_enabled(enabled);
    if due {
        items.request_frame();
    }
    if let Some(visibility) =
        world.get_resource::<render_scene::dropped_item::recording::ItemReplayVisibility>()
    {
        visibility.request(enabled, due);
    }
}
