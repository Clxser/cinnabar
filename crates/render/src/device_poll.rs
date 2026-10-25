//! The render world's single non-blocking device poll, after every submission of the frame.
//!
//! A poll takes the device's fence and lifetime locks that submits and buffer writes also need,
//! so owners never poll themselves: completion and map callbacks fire here, and readers consume
//! what arrived by the previous frame's poll. The poll runs before Bevy hands the frame to its
//! present thread, so it never waits behind the present for the queue's submission lock.

use std::sync::{Arc, Mutex, PoisonError};

use bevy::{
    prelude::{IntoScheduleConfigs, Res, ResMut, Resource, SubApp, SystemSet, warn},
    render::{
        Render, RenderSystems,
        render_resource::PollType,
        renderer::{RenderDevice, RenderQueue, render_system},
        view::window::{SurfacePresenter, present_frames},
    },
};
use web_time::Instant;

/// Owners whose systems register completion callbacks or request readbacks after the graph.
#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub struct FrameSubmissions;

/// Polls issued so far; tests read it to prove there is exactly one per frame.
#[derive(Resource, Default, Debug)]
pub(crate) struct DevicePolls(pub(crate) u64);

/// Idempotent, so every owner that relies on callbacks installs it.
pub(crate) fn install(render_app: &mut SubApp) {
    if render_app.world().contains_resource::<DevicePolls>() {
        return;
    }
    render_app
        .init_resource::<DevicePolls>()
        .configure_sets(
            Render,
            FrameSubmissions
                .in_set(RenderSystems::Render)
                .after(render_system),
        )
        .add_systems(
            Render,
            poll_device
                .in_set(RenderSystems::Render)
                .after(FrameSubmissions)
                .before(present_frames),
        );
}

/// Runs callback from a device poll after all submissions so far, including this frame, complete.
/// Call after the resource's last submission. Before rendering, only retire storage that
/// this frame no longer uses; FrameSubmissions systems may also retire this frame's storage.
pub(crate) fn on_frame_complete(queue: &RenderQueue, callback: impl FnOnce() + Send + 'static) {
    queue.on_submitted_work_done(callback);
}

/// Runs `callback(present_returned_at, gpu_completed_at)` once this frame's window presents have
/// returned and all submissions so far have completed, on whichever thread observes the later
/// event: Bevy's present thread or the render thread's device poll.
///
/// Register it where [`on_frame_complete`] would be, in [`FrameSubmissions`]. Presents may still be
/// running on the present thread then, so the present time is read when they return. Without a
/// [`SurfacePresenter`] nothing presents and the present time is the registration time.
/// `gpu_completed_at` is when completion was observed, and never precedes `present_returned_at`.
pub fn on_frame_presented_and_complete(
    queue: &RenderQueue,
    presenter: Option<&SurfacePresenter>,
    callback: impl FnOnce(Instant, Instant) + Send + 'static,
) {
    let join = Arc::new(Mutex::new(PresentedAndCompleted {
        present_returned_at: None,
        gpu_completed_at: None,
        callback: Some(callback),
    }));
    let presented = join.clone();
    let returned =
        move || PresentedAndCompleted::observe(&presented, |join| &mut join.present_returned_at);
    match presenter {
        Some(presenter) => presenter.after_presents(returned),
        None => returned(),
    }
    on_frame_complete(queue, move || {
        PresentedAndCompleted::observe(&join, |join| &mut join.gpu_completed_at);
    });
}

/// The two events [`on_frame_presented_and_complete`] waits for, and the callback to run.
struct PresentedAndCompleted<F> {
    present_returned_at: Option<Instant>,
    gpu_completed_at: Option<Instant>,
    callback: Option<F>,
}

impl<F: FnOnce(Instant, Instant)> PresentedAndCompleted<F> {
    /// Stamps one event now and runs the callback outside the lock once both have happened.
    fn observe(join: &Mutex<Self>, event: impl FnOnce(&mut Self) -> &mut Option<Instant>) {
        let ready = {
            let mut join = join.lock().unwrap_or_else(PoisonError::into_inner);
            *event(&mut join) = Some(Instant::now());
            match (join.present_returned_at, join.gpu_completed_at) {
                (Some(presented), Some(completed)) => join
                    .callback
                    .take()
                    .map(|callback| (callback, presented, completed.max(presented))),
                _ => None,
            }
        };
        if let Some((callback, presented, completed)) = ready {
            callback(presented, completed);
        }
    }
}

/// Reads a reserved queue index with an empty submit; callback registration may reserve one too.
#[cfg(test)]
pub(crate) fn submission_marker(queue: &RenderQueue) -> u64 {
    let index = format!("{:?}", queue.submit([]));
    index
        .chars()
        .filter(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .expect("submission index")
}

fn poll_device(device: Res<RenderDevice>, mut polls: ResMut<DevicePolls>) {
    #[cfg(feature = "tracy")]
    let _span = bevy::log::info_span!("render.device_poll").entered();
    if let Err(error) = device.poll(PollType::Poll) {
        warn!(?error, "could not nonblockingly poll the render device");
    }
    polls.0 += 1;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn present<F>(join: &mut PresentedAndCompleted<F>) -> &mut Option<Instant> {
        &mut join.present_returned_at
    }

    fn completion<F>(join: &mut PresentedAndCompleted<F>) -> &mut Option<Instant> {
        &mut join.gpu_completed_at
    }

    /// The callback runs once, after both events in either order, and completion is never
    /// stamped before the present returned.
    #[test]
    fn presented_and_completed_runs_once_after_both_events() {
        for completion_first in [false, true] {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let record = calls.clone();
            let join = Mutex::new(PresentedAndCompleted {
                present_returned_at: None,
                gpu_completed_at: None,
                callback: Some(move |presented: Instant, completed: Instant| {
                    record.lock().unwrap().push((presented, completed));
                }),
            });
            if completion_first {
                PresentedAndCompleted::observe(&join, completion);
                assert!(calls.lock().unwrap().is_empty());
                PresentedAndCompleted::observe(&join, present);
            } else {
                PresentedAndCompleted::observe(&join, present);
                assert!(calls.lock().unwrap().is_empty());
                PresentedAndCompleted::observe(&join, completion);
            }
            PresentedAndCompleted::observe(&join, completion);
            let calls = calls.lock().unwrap();
            assert_eq!(calls.len(), 1);
            assert!(calls[0].0 <= calls[0].1);
        }
    }
}
