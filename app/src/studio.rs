//! Native live recording coordination. IO, compression and encoding stay on bounded workers.

use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use bevy::prelude::*;
use client_shell::menu::MenuRuntime;
use crossbeam_channel::{Receiver, bounded};
use recording_studio::{
    MediaTime,
    archive::{RECORDING_VERSION, RecordingInfo},
    capture::{CapturePhase, CaptureRequest, CaptureSession, CaptureSettings, CaptureStatus},
};

use crate::{hud_tools::FrameCaptureSet, runtime::world::ClientWorld};

mod actors;
mod atmosphere;
mod audio;
mod composition;
mod environment;
mod export;
mod failures;
mod finalize;
mod frames;
mod observations;
mod playback;
mod preparation;
mod preview;
mod recovery;
mod targets;
mod terrain;
mod titles;
mod visuals;

#[derive(Resource, Default)]
struct NativeCapture {
    active: Option<Active>,
    startup: Option<Starting>,
    finishing: Option<Receiver<finalize::FinishResult>>,
    status: CaptureStatus,
    armed: bool,
    settings: CaptureSettings,
    cancel_start: bool,
}

struct Starting {
    result: Receiver<Result<CaptureSession, String>>,
    identity: (u64, i32, u64),
}

struct Active {
    session: CaptureSession,
    audio: Option<audio::AudioWorker>,
    terrain: Option<terrain::NativeTerrain>,
    native_failures: failures::NativeFailures,
    environment: Option<environment::NativeEnvironment>,
    actors: Option<actors::NativeActors>,
    targets: Option<targets::NativeTargets>,
    visuals: Option<visuals::NativeVisuals>,
    started: f64,
    identity: (u64, i32, u64),
    stopping: Option<(f64, MediaTime)>,
    video_readbacks: frames::Readbacks,
    captured: Arc<AtomicU64>,
    last_frame: Option<u64>,
    last_camera_frame: Option<u64>,
    last_attack: u64,
    last_combat: u64,
    failure: Option<String>,
}

/// Installs a normal feature, independent of developer-control and its fixed-clock recording mode.
pub(crate) fn configure(app: &mut App) {
    app.add_systems(
        Update,
        recovery::install
            .in_set(client_state::ClientFrameSet::UiPreparation)
            .before(preview::update),
    );
    app.init_resource::<client_presentation::actor_publication::recording::ReplayActors>();
    app.init_resource::<client_presentation::dropped_items::recording::ReplayItems>();
    app.add_plugins(render::replay::ReplayPreviewPlugin);
    app.init_resource::<render_scene::ui_preview::UiPreviewSurface>();
    app.init_resource::<playback::NativePlayback>().add_systems(
        Update,
        playback::update
            .in_set(client_state::ClientFrameSet::UiPreparation)
            .after(preview::update)
            .before(crate::ui_runtime::presentation::prepare_ui_runtime),
    );
    app.init_resource::<export::NativeExport>().add_systems(
        Update,
        export::update
            .in_set(client_state::ClientFrameSet::UiPreparation)
            .before(preview::update),
    );
    app.init_resource::<preview::NativePreview>().add_systems(
        Update,
        preview::update
            .in_set(client_state::ClientFrameSet::UiPreparation)
            .before(crate::ui_runtime::presentation::prepare_ui_runtime),
    );
    app.init_resource::<NativeCapture>().add_systems(
        Last,
        (update, terrain::observe).chain().before(FrameCaptureSet),
    );
    app.init_resource::<preparation::Cadence>()
        .add_systems(PreUpdate, preparation::request);
}

/// One short exclusive update transfers resource ownership; it never waits for worker completion.
fn update(world: &mut World) {
    if !world.resource::<NativeCapture>().status.phase.busy()
        && !world
            .get_resource::<MenuRuntime>()
            .is_some_and(MenuRuntime::studio_capture_pending)
    {
        world
            .resource_mut::<client_presentation::actor_publication::recording::ReplayActors>()
            .set_enabled(false);
        if let Some(mut particles) = world.get_resource_mut::<render_scene::ParticleSimulation>() {
            particles.set_recording(false);
        }
        visuals::scope(world, false);
        return;
    }
    let Some(mut state) = world.remove_resource::<NativeCapture>() else {
        return;
    };
    let now = world.resource::<Time<Real>>().elapsed_secs_f64();
    let request = world
        .get_resource_mut::<MenuRuntime>()
        .and_then(|mut menu| menu.take_studio_capture_request());
    let identity = observations::identity(world);
    if let Some(request) = request {
        handle_request(world, &mut state, request, now);
    }
    world
        .resource_mut::<client_presentation::actor_publication::recording::ReplayActors>()
        .set_enabled(state.armed || state.startup.is_some() || state.active.is_some());
    if let Some(mut particles) = world.get_resource_mut::<render_scene::ParticleSimulation>() {
        particles.set_recording(state.armed || state.startup.is_some() || state.active.is_some());
    }
    visuals::scope(
        world,
        state.settings.hud && (state.armed || state.startup.is_some() || state.active.is_some()),
    );
    poll_start(world, &mut state, identity, now);
    poll_finish(&mut state);
    if state.armed
        && state.startup.is_none()
        && identity.is_some()
        && world
            .get_resource::<MenuRuntime>()
            .is_some_and(|menu| !menu.is_visible() && !menu.is_connecting())
    {
        if let Err(error) = start(world, &mut state) {
            fail(&mut state, error);
        }
    }
    if let Some(mut active) = state.active.take() {
        let elapsed = MediaTime::from_seconds((now - active.started).max(0.0));
        if let Err(error) = frames::drain(&mut active) {
            failures::fatal(&mut active, error, now);
        }
        failures::poll(&mut active, now);
        if let Some(error) = active.session.error() {
            failures::fatal(&mut active, error, now);
        }
        if active
            .audio
            .as_ref()
            .is_some_and(audio::AudioWorker::failed)
        {
            failures::fatal(
                &mut active,
                "The game-audio recording worker stopped unexpectedly.".into(),
                now,
            );
        }
        if identity != Some(active.identity) || elapsed.is_err() {
            stop(&mut active, now);
        }
        if active.stopping.is_none() {
            let elapsed = elapsed.unwrap_or(MediaTime::ZERO);
            let sample_due = active
                .session
                .settings()
                .fps
                .frame_at(elapsed)
                .ok()
                .is_some_and(|frame| active.last_camera_frame != Some(frame));
            let sample_started = std::time::Instant::now();
            if let Err(error) = observations::capture(world, &mut active, elapsed) {
                failures::fatal(&mut active, error, now);
            } else {
                failures::sample(world, &mut active, elapsed, now);
                if active.stopping.is_none()
                    && let Err(error) = frames::capture(world, &mut active, elapsed)
                {
                    failures::fatal(&mut active, error, now);
                }
            }
            let sample_micros = if sample_due {
                sample_started
                    .elapsed()
                    .as_micros()
                    .min(u128::from(u64::MAX)) as u64
            } else {
                state.status.health.sample_micros
            };
            let actor_micros = world
                .get_resource::<client_presentation::actor_publication::recording::ReplayActors>()
                .map_or(0, |actors| actors.prepare_micros);
            let health = frames::health(&active, actor_micros, sample_micros);
            // Health counters do not force a fresh menu layout at the renderer's frame rate.
            if state.status.elapsed.0 / 500_000 != elapsed.0 / 500_000 {
                bevy::log::debug!(?health, "Studio capture health");
                state.status.health = health;
            }
            state.status.elapsed = MediaTime(elapsed.0 / 100_000 * 100_000);
            state.status.captured_frames = active.captured.load(Ordering::Acquire);
            state.status.missed_frames = active
                .session
                .settings()
                .fps
                .frames_covering(elapsed)
                .unwrap_or(0)
                .saturating_sub(state.status.captured_frames)
                .saturating_sub(active.video_readbacks.pending() as u64);
        }
        if let Some(message) = active
            .failure
            .as_deref()
            .or_else(|| active.native_failures.message())
            && state.status.message.as_deref() != Some(message)
        {
            state.status.message = Some(message.to_owned());
        }
        if let Some((since, duration)) = active.stopping {
            state.status.phase = CapturePhase::Finishing;
            state.status.elapsed = duration;
            if active.video_readbacks.pending() == 0 || now - since >= 5.0 {
                if active.video_readbacks.pending() != 0 {
                    active.failure.get_or_insert_with(|| "Video readback timed out while saving; the captured prefix was retained.".into());
                }
                finalize::start(&mut state, active, duration);
            } else {
                state.active = Some(active);
            }
        } else {
            state.active = Some(active);
        }
    }
    if let Some(mut menu) = world.get_resource_mut::<MenuRuntime>() {
        menu.set_studio_capture_status(state.status.clone());
    }
    world.insert_resource(state);
}

/// Arming from the launcher retains the current editor and uses the normal world-selection flow.
fn handle_request(world: &mut World, state: &mut NativeCapture, request: CaptureRequest, now: f64) {
    match request {
        CaptureRequest::Start(settings) if !state.status.phase.busy() => {
            if let Err(error) = settings.validate() {
                fail(state, error);
                return;
            }
            state.settings = settings;
            state.armed = true;
            state.cancel_start = false;
            state.status = CaptureStatus {
                phase: CapturePhase::Armed,
                fps: settings.fps,
                ..Default::default()
            };
            if let Some(mut menu) = world.get_resource_mut::<MenuRuntime>() {
                if observations::identity_from_menu(&menu) {
                    menu.show_world();
                } else {
                    menu.activate(launcher::menu::MenuAction::Navigate(
                        launcher::menu::MenuScreen::Play,
                    ));
                }
            }
        }
        CaptureRequest::Stop => {
            state.armed = false;
            state.cancel_start = true;
            if let Some(active) = &mut state.active {
                stop(active, now);
            } else if state.startup.is_none() && state.finishing.is_none() {
                state.status = CaptureStatus::default();
            }
        }
        CaptureRequest::Bookmark => {
            if let Some(active) = &mut state.active
                && active.stopping.is_none()
            {
                let at = MediaTime::from_seconds((now - active.started).max(0.0))
                    .unwrap_or(MediaTime::ZERO);
                if let Err(error) = active.session.event(
                    at,
                    recording_studio::automation::GameplayEventKind::Bookmark {
                        label: "Bookmark".into(),
                    },
                ) {
                    failures::fatal(active, error, now);
                }
            }
        }
        CaptureRequest::Start(_) => {}
    }
}

/// Startup uses a worker even for directory reservation, FFmpeg discovery and driver probing.
fn start(world: &mut World, state: &mut NativeCapture) -> Result<(), String> {
    let identity = observations::identity(world).ok_or("Enter a world before recording.")?;
    let local_actor = identity.2;
    let root = world
        .resource::<MenuRuntime>()
        .layout()
        .user_data_root
        .join("studio");
    let size = world
        .query_filtered::<&Window, With<bevy::window::PrimaryWindow>>()
        .single(world)
        .map(|window| {
            [
                window.physical_width().max(2),
                window.physical_height().max(2),
            ]
        })
        .map_err(|_| "The game window is unavailable.".to_owned())?;
    let created = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_millis();
    let info = RecordingInfo {
        version: RECORDING_VERSION,
        name: "Recording".into(),
        game_version: protocol::GAME_VERSION.into(),
        protocol_version: protocol::PROTOCOL_VERSION as u32,
        scene_schema: String::new(),
        created_unix_ms: created.min(u128::from(u64::MAX)) as u64,
        viewport: size,
        local_actor,
    };
    let (completed, startup) = bounded(1);
    let settings = state.settings;
    std::thread::Builder::new()
        .name("studio-capture-start".into())
        .spawn(move || {
            let _ = completed.send(CaptureSession::create(&root, info, settings));
        })
        .map_err(|error| error.to_string())?;
    state.startup = Some(Starting {
        result: startup,
        identity,
    });
    state.armed = false;
    state.status.phase = CapturePhase::Starting;
    Ok(())
}

/// A late startup result cannot begin capturing after cancellation or leaving the world.
fn poll_start(
    world: &mut World,
    state: &mut NativeCapture,
    identity: Option<(u64, i32, u64)>,
    now: f64,
) {
    let same_world = state
        .startup
        .as_ref()
        .is_some_and(|startup| Some(startup.identity) == identity);
    if same_world
        && !state.cancel_start
        && world
            .get_resource::<MenuRuntime>()
            .is_some_and(MenuRuntime::is_visible)
    {
        return;
    }
    let Some(result) = state
        .startup
        .as_ref()
        .and_then(|startup| match startup.result.try_recv() {
            Ok(result) => Some(result),
            Err(crossbeam_channel::TryRecvError::Empty) => None,
            Err(crossbeam_channel::TryRecvError::Disconnected) => Some(Err(
                "The recording startup worker stopped unexpectedly.".into(),
            )),
        })
    else {
        return;
    };
    state.startup = None;
    match result {
        Ok(session) if !state.cancel_start && same_world => {
            let actors = match actors::start(world, &session) {
                Ok(actors) => actors,
                Err(error) => {
                    fail(state, error);
                    return;
                }
            };
            let environment = match environment::start(world, &session) {
                Ok(environment) => environment,
                Err(error) => {
                    fail(state, error);
                    return;
                }
            };
            let terrain = match terrain::start(world, &session) {
                Ok(terrain) => terrain,
                Err(error) => {
                    fail(state, error);
                    return;
                }
            };
            let requested_audio = session.settings().game_audio;
            let visuals = match visuals::start(world, &session) {
                Ok(visuals) => visuals,
                Err(error) => {
                    fail(state, error);
                    return;
                }
            };
            let targets = match targets::start(world, &session) {
                Ok(targets) => targets,
                Err(error) => {
                    fail(state, error);
                    return;
                }
            };
            let audio = if requested_audio {
                world
                    .get_non_send_mut::<client_audio::named_audio::AudioDevice>()
                    .and_then(|mut device| device.start_live_capture())
                    .and_then(|tap| {
                        match audio::AudioWorker::start(
                            tap,
                            session
                                .directory()
                                .join(recording_studio::capture::AUDIO_FILE),
                        ) {
                            Ok(audio) => Some(audio),
                            Err(error) => {
                                state.status.message =
                                    Some(format!("Recording without game audio: {error}"));
                                None
                            }
                        }
                    })
            } else {
                None
            };
            if requested_audio && audio.is_none() && state.status.message.is_none() {
                state.status.message = Some(
                    "Recording without game audio: the audio device or capture tap is unavailable."
                        .into(),
                );
            }
            state.status.phase = CapturePhase::Recording;
            state.status.path = Some(session.directory().to_path_buf());
            state.status.audio = audio.is_some();
            bevy::log::info!(game_audio = state.status.audio, "Studio recording started");
            state.active = Some(Active {
                session,
                audio,
                terrain: Some(terrain),
                native_failures: Default::default(),
                environment: Some(environment),
                actors: Some(actors),
                targets: Some(targets),
                visuals: Some(visuals),
                started: now,
                identity: identity.expect("checked world identity"),
                stopping: None,
                video_readbacks: frames::Readbacks::default(),
                captured: Arc::default(),
                last_frame: None,
                last_camera_frame: None,
                last_attack: observations::attack_revision(world),
                last_combat: observations::combat_revision(world),
                failure: None,
            });
        }
        Ok(session) => {
            drop(session);
            state.status = CaptureStatus::default();
        }
        Err(error) => fail(state, error),
    }
}

/// Quantizes both tracks to one final video-frame boundary before draining callbacks.
fn stop(active: &mut Active, now: f64) {
    if active.stopping.is_some() {
        return;
    }
    let fps = active.session.settings().fps;
    let elapsed = MediaTime::from_seconds((now - active.started).max(0.0))
        .unwrap_or(MediaTime(recording_studio::time::MAX_MEDIA_MICROS));
    let frames = fps
        .frames_covering(elapsed)
        .unwrap_or(1)
        .max(active.last_frame.map_or(1, |frame| frame + 1));
    let duration = fps.frame_time(frames).unwrap_or(elapsed);
    stop_at(active, now, duration);
}

/// A failed native sample is an exclusive boundary; never extend it to a future video frame.
fn stop_at(active: &mut Active, now: f64, duration: MediaTime) {
    let (since, duration) = active
        .stopping
        .map_or((now, duration), |(since, previous)| {
            (since, duration.min(previous))
        });
    if let Some(audio) = &active.audio {
        audio.stop(duration);
    }
    active.stopping = Some((since, duration));
    bevy::log::info!(duration_us = duration.0, error = ?active.failure, "Studio recording stopping");
}

/// Final results are polled without filesystem reads or worker joins.
fn poll_finish(state: &mut NativeCapture) {
    let Some(result) = state
        .finishing
        .as_ref()
        .and_then(|finished| match finished.try_recv() {
            Ok(result) => Some(result),
            Err(crossbeam_channel::TryRecvError::Empty) => None,
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                Some(Err("The recording finalizer stopped unexpectedly."
                    .to_owned()
                    .into()))
            }
        })
    else {
        return;
    };
    state.finishing = None;
    match result {
        Ok(summary) => {
            bevy::log::info!(
                frames = summary.manifest.frames,
                video = summary.video.is_some(),
                missed_frames = summary.manifest.missed_video_frames,
                duration_us = summary.manifest.duration.0,
                "Studio recording saved"
            );
            state.status.phase = CapturePhase::Complete;
            state.status.captured_frames = summary
                .manifest
                .frames
                .saturating_sub(summary.manifest.missed_video_frames);
            state.status.missed_frames = summary.manifest.missed_video_frames;
            state.status.fps = summary.manifest.settings.fps;
            state.status.path = Some(summary.directory);
            state.status.elapsed = summary.manifest.duration;
            state.status.message = summary
                .manifest
                .scene_tracks
                .iter()
                .find(|track| !track.complete)
                .map(|track| {
                    format!(
                        "Video saved; 3D replay is incomplete: {}",
                        track
                            .error
                            .as_deref()
                            .unwrap_or("scene capture did not finish")
                    )
                });
        }
        Err(error) => {
            if let Some(prefix) = error.verified_prefix {
                state.status.elapsed = prefix;
            }
            fail(state, error.message);
        }
    }
}

/// A failed optional studio operation never tears down a live network session or world.
fn fail(state: &mut NativeCapture, message: String) {
    bevy::log::warn!(error = %message, "Studio recording failed");
    state.armed = false;
    state.status.phase = CapturePhase::Failed;
    state.status.message = Some(message);
}
