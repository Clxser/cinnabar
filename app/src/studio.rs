//! Native live recording coordination. IO, compression and encoding stay on bounded workers.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use bevy::prelude::*;
use client_shell::menu::MenuRuntime;
use crossbeam_channel::{Receiver, bounded};
use recording_studio::{
    MediaTime,
    archive::{RECORDING_VERSION, RecordingInfo},
    capture::{
        CapturePhase, CaptureRequest, CaptureSession, CaptureSettings, CaptureStatus,
        CaptureSummary,
    },
};

use crate::{hud_tools::FrameCaptureSet, runtime::world::ClientWorld};

mod actors;
mod audio;
mod environment;
mod export;
mod frames;
mod observations;
mod preview;
mod targets;
mod terrain;

#[derive(Resource, Default)]
struct NativeCapture {
    active: Option<Active>,
    startup: Option<Starting>,
    finishing: Option<Receiver<Result<CaptureSummary, String>>>,
    status: CaptureStatus,
    armed: bool,
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
    environment: Option<environment::NativeEnvironment>,
    actors: Option<actors::NativeActors>,
    targets: Option<targets::NativeTargets>,
    started: f64,
    identity: (u64, i32, u64),
    stopping: Option<(f64, MediaTime)>,
    pending: Arc<AtomicBool>,
    captured: Arc<AtomicU64>,
    failed: Receiver<String>,
    errors: crossbeam_channel::Sender<String>,
    last_frame: Option<u64>,
    last_attack: u64,
    last_combat: u64,
    failure: Option<String>,
}

/// Installs a normal feature, independent of developer-control and its fixed-clock recording mode.
pub(crate) fn configure(app: &mut App) {
    app.add_plugins(render::replay::ReplayPreviewPlugin);
    app.init_resource::<render_scene::ui_preview::UiPreviewSurface>();
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
}

/// One short exclusive update transfers resource ownership; it never waits for worker completion.
fn update(world: &mut World) {
    if !world.resource::<NativeCapture>().status.phase.busy()
        && !world
            .get_resource::<MenuRuntime>()
            .is_some_and(MenuRuntime::studio_capture_pending)
    {
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
        if let Some(error) = active
            .terrain
            .as_mut()
            .and_then(|terrain| terrain.capture.error())
        {
            active.failure.get_or_insert(error);
            stop(&mut active, now);
        }
        if let Some(error) = active
            .environment
            .as_ref()
            .and_then(|environment| environment.capture.error())
        {
            active.failure.get_or_insert(error);
            stop(&mut active, now);
        }
        if let Some(error) = active
            .failed
            .try_recv()
            .ok()
            .or_else(|| active.session.error())
        {
            active.failure.get_or_insert(error);
            stop(&mut active, now);
        }
        if let Some(error) = active
            .actors
            .as_ref()
            .and_then(|actors| actors.capture.error())
        {
            active.failure.get_or_insert(error);
            stop(&mut active, now);
        }
        if let Some(error) = active
            .targets
            .as_ref()
            .and_then(|targets| targets.capture.error())
        {
            active.failure.get_or_insert(error);
            stop(&mut active, now);
        }
        if active
            .audio
            .as_ref()
            .is_some_and(audio::AudioWorker::failed)
        {
            active.failure.get_or_insert_with(|| {
                "The game-audio recording worker stopped unexpectedly.".into()
            });
            stop(&mut active, now);
        }
        if identity != Some(active.identity) || elapsed.is_err() {
            stop(&mut active, now);
        }
        if active.stopping.is_none() {
            let elapsed = elapsed.unwrap_or(MediaTime::ZERO);
            if let Err(error) = observations::capture(world, &mut active, elapsed)
                .and_then(|()| environment::sample(world, &mut active, elapsed))
                .and_then(|()| actors::sample(world, &mut active, elapsed))
                .and_then(|()| targets::sample(world, &mut active, elapsed))
                .and_then(|()| frames::capture(world, &mut active, elapsed))
            {
                active.failure.get_or_insert(error);
                stop(&mut active, now);
            }
            state.status.elapsed = MediaTime(elapsed.0 / 100_000 * 100_000);
            state.status.captured_frames = active.captured.load(Ordering::Acquire);
            state.status.missed_frames = active
                .session
                .settings()
                .fps
                .frame_at(elapsed)
                .unwrap_or(0)
                .saturating_sub(state.status.captured_frames);
        }
        if let Some((since, duration)) = active.stopping {
            state.status.phase = CapturePhase::Finishing;
            state.status.elapsed = duration;
            if !active.pending.load(Ordering::Acquire) || now - since >= 5.0 {
                finalize(&mut state, active, duration);
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
        CaptureRequest::Start if !state.status.phase.busy() => {
            state.armed = true;
            state.cancel_start = false;
            state.status = CaptureStatus {
                phase: CapturePhase::Armed,
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
                    active.failure.get_or_insert(error);
                    stop(active, now);
                }
            }
        }
        CaptureRequest::Start => {}
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
    std::thread::Builder::new()
        .name("studio-capture-start".into())
        .spawn(move || {
            let _ = completed.send(CaptureSession::create(
                &root,
                info,
                CaptureSettings::default(),
            ));
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
            let (errors, failed) = bounded(1);
            state.active = Some(Active {
                session,
                audio,
                terrain: Some(terrain),
                environment: Some(environment),
                actors: Some(actors),
                targets: Some(targets),
                started: now,
                identity: identity.expect("checked world identity"),
                stopping: None,
                pending: Arc::default(),
                captured: Arc::default(),
                failed,
                errors,
                last_frame: None,
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
    if let Some(audio) = &active.audio {
        audio.stop(duration);
    }
    active.stopping = Some((now, duration));
}

/// Moves every blocking finish operation away from gameplay, retaining a visible finishing state.
fn finalize(state: &mut NativeCapture, mut active: Active, duration: MediaTime) {
    let (completed, finishing) = bounded(1);
    let result = std::thread::Builder::new()
        .name("studio-capture-finish".into())
        .spawn(move || {
            let targets_error = if let Some(targets) = active.targets {
                let error = targets.capture.finish(duration).err();
                let metadata = recording_studio::capture::CaptureTrack {
                    schema: recording_scene::targets::TARGET_TRACK_SCHEMA.into(),
                    directory: recording_scene::targets::TARGET_DIRECTORY.into(),
                    complete: error.is_none(),
                    error: error
                        .as_ref()
                        .map(|error| error.chars().take(512).collect()),
                };
                active.session.scene_track(metadata).err().or(error)
            } else {
                None
            };
            let actors_error = if let Some(actors) = active.actors {
                let error = actors.capture.finish(duration).err();
                let metadata = recording_studio::capture::CaptureTrack {
                    schema: recording_scene::actors::ACTOR_TRACK_SCHEMA.into(),
                    directory: recording_scene::actors::ACTOR_DIRECTORY.into(),
                    complete: error.is_none(),
                    error: error
                        .as_ref()
                        .map(|error| error.chars().take(512).collect()),
                };
                active.session.scene_track(metadata).err().or(error)
            } else {
                None
            };
            let environment_error = if let Some(environment) = active.environment {
                let error = environment.capture.finish(duration).err();
                let metadata = recording_studio::capture::CaptureTrack {
                    schema: recording_scene::environment::ENVIRONMENT_TRACK_SCHEMA.into(),
                    directory: recording_scene::environment::ENVIRONMENT_DIRECTORY.into(),
                    complete: error.is_none(),
                    error: error
                        .as_ref()
                        .map(|error| error.chars().take(512).collect()),
                };
                active.session.scene_track(metadata).err().or(error)
            } else {
                None
            };
            let terrain_error = if let Some(terrain) = active.terrain {
                let error = terrain.capture.finish(duration).err();
                let metadata = recording_studio::capture::CaptureTrack {
                    schema: recording_scene::TERRAIN_TRACK_SCHEMA.into(),
                    directory: recording_scene::journal::TERRAIN_DIRECTORY.into(),
                    complete: error.is_none(),
                    error: error
                        .as_ref()
                        .map(|error| error.chars().take(512).collect()),
                };
                active.session.scene_track(metadata).err().or(error)
            } else {
                None
            };
            let audio_result = active.audio.map(audio::AudioWorker::finish).transpose();
            let (audio, audio_error) = match audio_result {
                Ok(audio) => (audio, None),
                Err(error) => (None, Some(error)),
            };
            let result = {
                let expected = active
                    .session
                    .settings()
                    .fps
                    .frames_covering(duration)
                    .unwrap_or(0);
                let missed = expected.saturating_sub(active.captured.load(Ordering::Acquire));
                let summary = active.session.finish(
                    duration,
                    audio.as_ref().map(|(path, _)| path.as_path()),
                    missed,
                    audio.as_ref().map_or(0, |(_, dropped)| *dropped),
                );
                match (
                    summary,
                    active
                        .failure
                        .or(audio_error)
                        .or(terrain_error)
                        .or(environment_error)
                        .or(actors_error)
                        .or(targets_error),
                ) {
                    (Ok(summary), None) => Ok(summary),
                    (Ok(_), Some(error)) => {
                        Err(format!("Recording saved with an interruption: {error}"))
                    }
                    (Err(error), _) => Err(error),
                }
            };
            let _ = completed.send(result);
        });
    match result {
        Ok(_) => state.finishing = Some(finishing),
        Err(error) => fail(state, error.to_string()),
    }
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
                Some(Err("The recording finalizer stopped unexpectedly.".into()))
            }
        })
    else {
        return;
    };
    state.finishing = None;
    match result {
        Ok(summary) => {
            state.status.phase = CapturePhase::Complete;
            state.status.path = Some(summary.directory);
            state.status.elapsed = summary.manifest.duration;
            state.status.message = None;
        }
        Err(error) => fail(state, error),
    }
}

/// A failed optional studio operation never tears down a live network session or world.
fn fail(state: &mut NativeCapture, message: String) {
    state.armed = false;
    state.status.phase = CapturePhase::Failed;
    state.status.message = Some(message);
}
