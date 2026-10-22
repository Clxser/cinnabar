//! Audible Studio playback is paced by the device sample cursor and admitted displayed video frames.

use bevy::prelude::*;
use client_audio::{named_audio::AudioDevice, preview::PreviewOutput};
use client_shell::menu::MenuRuntime;
use recording_studio::audio::{MonitorClock, PreviewRequest, PreviewStatus};

mod worker;

struct PlaybackKey {
    epoch: [u64; 3],
    sample_rate: u32,
    project: std::sync::Arc<recording_studio::Project>,
    source_base: std::path::PathBuf,
    speed: recording_studio::playback::monitoring::MonitoringSpeed,
    clock: MonitorClock,
}

impl PlaybackKey {
    /// Unsaved edit previews and Save As path changes must also retire the old PCM queue.
    fn new(request: &PreviewRequest, sample_rate: u32) -> Result<Self, String> {
        Ok(Self {
            epoch: request.preview.epoch,
            sample_rate,
            project: request.preview.project.clone(),
            source_base: request.preview.source_base.clone(),
            speed: request.speed,
            clock: MonitorClock::new(
                request.preview.at,
                request.duration,
                request.speed.rate(),
                sample_rate,
            )?,
        })
    }

    /// Camera navigation, preview quality and forward time do not change the audible source history.
    fn matches(&self, request: &PreviewRequest, sample_rate: u32) -> bool {
        self.epoch == request.preview.epoch
            && self.speed == request.speed
            && self.sample_rate == sample_rate
            && std::sync::Arc::ptr_eq(&self.project, &request.preview.project)
            && self.source_base == request.preview.source_base
    }
}

#[derive(Resource, Default)]
pub(super) struct NativePlayback {
    output: Option<PreviewOutput>,
    worker: Option<worker::Worker>,
    key: Option<PlaybackKey>,
    generation: u64,
    job: u64,
    failed: Option<PreviewStatus>,
    exclusive: bool,
    suspending: bool,
}

/// Main-thread work consists of bounded message polling, atomic device control and immutable metadata clones.
pub(super) fn update(
    mut state: ResMut<NativePlayback>,
    mut menu: ResMut<MenuRuntime>,
    device: Option<NonSendMut<AudioDevice>>,
    settings: Option<Res<client_audio::audio::AudioSettings>>,
) {
    let exclusive = menu.studio_audio_exclusive();
    if !exclusive && !state.exclusive && !state.suspending {
        return;
    }
    state.exclusive = exclusive;
    if let Some(output) = &state.output {
        output.set_exclusive(exclusive);
    }
    let request = menu.studio_audio_request();
    let Some(mut device) = device else {
        stop(&mut state);
        if let Some(request) = request {
            menu.set_studio_audio_status(failure(
                &request,
                "The audio output device is unavailable.".into(),
            ));
        } else {
            menu.set_studio_audio_status(PreviewStatus::default());
        }
        return;
    };
    let output = device.preview_output();
    if state
        .output
        .as_ref()
        .is_some_and(|old| !old.same_output(&output))
    {
        stop(&mut state);
    }
    output.set_exclusive(exclusive);
    output.set_gain(settings.as_ref().map_or(1.0, |settings| {
        settings.volume(assets::audio_category::AudioCategory::Master)
    }));
    state.output = Some(output.clone());
    if request.is_none() || state.suspending {
        if device.preview_available() {
            retire_paused(&mut state, &mut menu);
        } else {
            stop(&mut state);
            menu.set_studio_audio_status(PreviewStatus::default());
        }
        return;
    }
    let request = request.expect("visible audio request admitted");
    if state
        .key
        .as_ref()
        .is_none_or(|key| !key.matches(&request, output.sample_rate()))
    {
        stop(&mut state);
        match PlaybackKey::new(&request, output.sample_rate()) {
            Ok(key) => state.key = Some(key),
            Err(error) => {
                fail(&mut state, &mut menu, &request, error);
                return;
            }
        }
        state.failed = None;
    }
    if let Some(failed) = &state.failed {
        menu.set_studio_audio_status(failed.clone());
        return;
    }
    if !device.preview_available() {
        if device.preview_opening() {
            menu.set_studio_audio_status(PreviewStatus {
                epoch: Some(request.preview.epoch),
                engaged: true,
                buffering: true,
                position: request.preview.at,
                ..Default::default()
            });
        } else {
            let status = failure(
                &request,
                "The default audio device could not be opened. Audio preview is unavailable."
                    .into(),
            );
            state.failed = Some(status.clone());
            menu.set_studio_audio_status(status);
        }
        return;
    }
    if state.worker.is_none() {
        match worker::Worker::start() {
            Ok(worker) => state.worker = Some(worker),
            Err(error) => {
                let status = failure(&request, error);
                state.failed = Some(status.clone());
                menu.set_studio_audio_status(status);
                return;
            }
        }
    }
    if state.generation == 0 {
        let clock = state.key.as_ref().expect("audio clock installed").clock;
        let result = (|| {
            let generation = output.reset(0, clock.end_frame());
            state.generation = generation;
            state.job = state
                .worker
                .as_mut()
                .expect("audio worker installed")
                .submit(request.clone(), output.clone(), generation)?;
            Ok(())
        })();
        if let Err(error) = result {
            fail(&mut state, &mut menu, &request, error);
            return;
        }
    }
    if let Some(error) = state
        .worker
        .as_ref()
        .and_then(|worker| worker.error(state.job))
    {
        fail(&mut state, &mut menu, &request, error);
        return;
    }
    let ceiling = request.ceiling.unwrap_or(request.preview.at);
    let clock = state.key.as_ref().expect("audio clock installed").clock;
    let ceiling = clock.frame(ceiling);
    output.set_playing(request.playing && request.ceiling.is_some(), ceiling);
    let position = clock.time(output.position());
    let buffered_until = clock.time(output.buffered_until());
    let paused_position = output
        .paused_position()
        .map(|position| clock.time(position));
    menu.set_studio_audio_status(PreviewStatus {
        epoch: Some(request.preview.epoch),
        engaged: true,
        ready: output.buffered_until() > output.position() || position >= request.duration,
        buffering: request.playing
            && (output.waiting() || output.buffered_until() <= output.position()),
        position,
        paused_position,
        buffered_until,
        underruns: output.underruns(),
        error: None,
    });
}

/// Hidden editor routes cancel decoding immediately, then retire only after the callback stops.
fn retire_paused(state: &mut NativePlayback, menu: &mut MenuRuntime) {
    if let Some(worker) = &mut state.worker {
        worker.cancel();
    }
    if let Some((key, output)) = state.key.as_ref().zip(state.output.as_ref())
        && state.generation != 0
    {
        state.suspending = true;
        output.set_playing(false, output.position());
        let position = key.clock.time(output.position());
        let paused_position = output
            .paused_position()
            .map(|position| key.clock.time(position));
        menu.set_studio_audio_status(PreviewStatus {
            epoch: Some(key.epoch),
            engaged: true,
            position,
            paused_position,
            ..Default::default()
        });
        if paused_position.is_none() {
            return;
        }
    }
    stop(state);
    menu.set_studio_audio_status(PreviewStatus::default());
}

/// Teardown never waits for worker IO and invalidates callback-held blocks immediately.
fn stop(state: &mut NativePlayback) {
    state.suspending = false;
    if state.key.is_none() && state.generation == 0 {
        return;
    }
    if let Some(worker) = &mut state.worker {
        worker.cancel();
    }
    if let Some(output) = &state.output {
        output.reset(0, 0);
    }
    state.generation = 0;
    state.job = 0;
    state.key = None;
    state.failed = None;
}

/// A failure is a separate truthful audio status, so image preview and recording diagnostics remain intact.
fn failure(request: &PreviewRequest, error: String) -> PreviewStatus {
    PreviewStatus {
        epoch: Some(request.preview.epoch),
        position: request.preview.at,
        error: Some(error),
        ..Default::default()
    }
}

/// Stop output before publishing an error; only an explicit later Play may continue silently.
fn fail(
    state: &mut NativePlayback,
    menu: &mut MenuRuntime,
    request: &PreviewRequest,
    error: String,
) {
    if let Some(output) = &state.output {
        output.set_playing(false, output.position());
    }
    if let Some(worker) = &mut state.worker {
        worker.cancel();
    }
    let status = failure(request, error);
    state.failed = Some(status.clone());
    menu.set_studio_audio_status(status);
}

impl Drop for NativePlayback {
    fn drop(&mut self) {
        if let Some(output) = &self.output {
            output.set_playing(false, output.position());
            output.set_exclusive(false);
        }
    }
}

#[cfg(test)]
mod tests;
