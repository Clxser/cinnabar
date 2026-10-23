//! Native editor preview bridge. Frame preparation, process IO and pixel hashing stay on one worker.

use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

use bevy::prelude::*;
use client_shell::{menu::MenuRuntime, ui_resources::UiPresentationResource};
use recording_studio::preview::{CameraPreview, PreviewRequest, PreviewStatus, RasterPreview};
use render::replay::ReplayRenderContext;
use render_model::{UI_STUDIO_PREVIEW_MAX_SIZE, UiTexturePage};
use render_scene::ui_preview::{PreviewImage, UiPreviewImage, UiPreviewSurface};

mod metrics;
pub(super) mod scene;
#[cfg(test)]
mod tests;

#[derive(Resource, Default)]
pub(super) struct NativePreview {
    worker: Option<Worker>,
    identity: Option<RequestIdentity>,
    requested: Option<u64>,
    failed: Option<RequestIdentity>,
    font: Option<Arc<assets::RuntimeFontCatalog>>,
    displayed: Option<PreviewStatus>,
    displayed_identity: Option<RequestIdentity>,
    displayed_generation: Option<u64>,
    prepared: Option<Completed>,
}

impl NativePreview {
    /// A paused view can keep its exact admitted image while an unused future frame is retired.
    fn has_frame(
        &self,
        request: &PreviewRequest,
        at: recording_studio::MediaTime,
        generation: u64,
    ) -> bool {
        self.displayed_generation == Some(generation)
            && self.displayed.as_ref().is_some_and(|status| {
                status.ready
                    && status.at == at
                    && status.epoch == Some(request.epoch)
                    && status.error.is_none()
            })
            && self
                .displayed_identity
                .as_ref()
                .is_some_and(|identity| identity.matches(request))
    }
}

/// Time advances coalesce; document edits and inspection views cancel stale in-flight frames.
#[derive(Clone)]
struct RequestIdentity {
    quality: recording_studio::preview::PreviewQuality,
    cadence: Option<recording_studio::FrameRate>,
    epoch: [u64; 3],
    project: Arc<recording_studio::Project>,
    source_base: std::path::PathBuf,
    camera_view: Option<recording_studio::camera::CameraView>,
}

impl RequestIdentity {
    /// Retains only immutable request owners; changing playback time does not change identity.
    fn new(request: &PreviewRequest) -> Self {
        Self {
            quality: request.quality,
            cadence: request.cadence,
            epoch: request.epoch,
            project: Arc::clone(&request.project),
            source_base: request.source_base.clone(),
            camera_view: request.camera_view,
        }
    }

    /// Runtime project replacement can update a drag without incrementing a saved edit revision.
    fn matches(&self, request: &PreviewRequest) -> bool {
        self.epoch == request.epoch
            && self.quality == request.quality
            && self.cadence == request.cadence
            && Arc::ptr_eq(&self.project, &request.project)
            && self.source_base == request.source_base
            && self.camera_view == request.camera_view
    }

    /// A camera draft can finish one frame behind input; only source/edit/seek changes invalidate it.
    fn same_generation(&self, request: &PreviewRequest) -> bool {
        self.epoch == request.epoch
            && self.source_base == request.source_base
            && self.quality == request.quality
            && self.cadence == request.cadence
    }
}

struct Job {
    generation: u64,
    request: PreviewRequest,
    font: Arc<assets::RuntimeFontCatalog>,
    maximum: [u32; 2],
}
struct Completed {
    generation: u64,
    at: recording_studio::MediaTime,
    identity: Option<RequestIdentity>,
    result: Result<PreviewOutput, String>,
}

struct PreviewOutput {
    page: UiTexturePage,
    image: Option<Arc<PreviewImage>>,
    notice: Option<String>,
    camera: Option<CameraPreview>,
}

struct Shared {
    pending: Mutex<Option<Job>>,
    changed: Condvar,
    completed: Mutex<Option<Completed>>,
    generation: AtomicU64,
    stopped: AtomicBool,
}
struct Worker {
    shared: Arc<Shared>,
}

/// Does no work during normal play, and never waits on a preview process or worker lock.
pub(super) fn update(
    mut state: ResMut<NativePreview>,
    mut menu: ResMut<MenuRuntime>,
    mut presentation: ResMut<UiPresentationResource>,
    mut surface: ResMut<UiPreviewSurface>,
    gpu: Option<Res<ReplayRenderContext>>,
) {
    let state = &mut *state;
    let Some(request) = menu.studio_preview_request() else {
        if state.worker.is_some() || state.identity.is_some() || state.failed.is_some() {
            state.worker.take();
            state.identity = None;
            state.requested = None;
            state.failed = None;
            state.font = None;
            state.displayed = None;
            state.displayed_identity = None;
            state.displayed_generation = None;
            state.prepared = None;
            presentation.set_studio_preview_page(None);
            surface.0 = None;
            menu.set_studio_preview_status(PreviewStatus::default());
        }
        return;
    };
    let playing = menu.studio_preview_playing();
    let (request, maximum) = request.for_interactive(UI_STUDIO_PREVIEW_MAX_SIZE, playing);
    let request = match request.with_monitoring_speed(menu.studio_monitoring_speed()) {
        Ok(request) => request,
        Err(error) => {
            menu.set_studio_preview_status(PreviewStatus {
                error: Some(error),
                ..Default::default()
            });
            return;
        }
    };
    let due = request.frame_time().unwrap_or(request.at);
    if state
        .failed
        .as_ref()
        .is_some_and(|failed| failed.matches(&request))
    {
        return;
    }
    state.failed = None;
    if state
        .worker
        .as_ref()
        .is_some_and(|worker| worker.shared.stopped.load(Ordering::Acquire))
    {
        let error = state
            .worker
            .as_ref()
            .and_then(Worker::poll)
            .and_then(|completed| completed.result.err())
            .unwrap_or_else(|| {
                "The preview worker stopped unexpectedly. Seek again to retry.".into()
            });
        state.worker.take();
        state.failed = Some(RequestIdentity::new(&request));
        menu.set_studio_preview_status(PreviewStatus {
            error: Some(error),
            ..Default::default()
        });
        return;
    }
    if state.worker.is_none() {
        match Worker::start(gpu.as_deref().cloned()) {
            Ok(worker) => state.worker = Some(worker),
            Err(error) => {
                state.failed = Some(RequestIdentity::new(&request));
                menu.set_studio_preview_status(PreviewStatus {
                    error: Some(error),
                    ..Default::default()
                });
                return;
            }
        }
    }
    let (font, changed_font) = retained_font(state.font.as_ref(), presentation.font_snapshot());
    if changed_font {
        state.font = Some(font.clone());
    }
    let cancel_inflight = state
        .identity
        .as_ref()
        .is_none_or(|old| !old.same_generation(&request))
        || changed_font;
    let changed_identity = state
        .identity
        .as_ref()
        .is_none_or(|old| !old.matches(&request))
        || changed_font;
    if changed_identity {
        state.identity = Some(RequestIdentity::new(&request));
        state.requested = None;
        state.prepared = None;
        if cancel_inflight && let Some(worker) = &state.worker {
            worker.shared.generation.fetch_add(1, Ordering::AcqRel);
        }
        let generation = state
            .worker
            .as_ref()
            .map(|worker| worker.shared.generation.load(Ordering::Acquire));
        let status = refresh_status(
            state.displayed.as_ref(),
            request.epoch,
            cancel_inflight || state.displayed_generation != generation,
        );
        menu.set_studio_preview_status(status);
    }
    let worker = state.worker.as_ref().expect("preview worker installed");
    let generation = worker.shared.generation.load(Ordering::Acquire);
    let completed = state
        .prepared
        .take()
        .or_else(|| worker.poll())
        .filter(|completed| completed.generation == generation)
        .filter(|completed| {
            state.displayed_generation != Some(generation)
                || follows_displayed(
                    completed.at,
                    state.displayed.as_ref().map(|status| status.at),
                    playing,
                )
        });
    let completed = completed.and_then(|completed| match delivery(completed.at, due, playing) {
        Delivery::Show => Some(completed),
        Delivery::Hold => {
            state.prepared = Some(completed);
            None
        }
        Delivery::Discard => {
            if state.requested == request.frame_rate().frame_at(completed.at).ok() {
                state.requested = if state.has_frame(&request, due, generation) {
                    request.frame_rate().frame_at(due).ok()
                } else {
                    None
                };
            }
            None
        }
    });
    if let Some(completed) = completed {
        let identity = completed.identity;
        let status = match completed.result {
            Ok(output) => {
                let identity = output.page.identity();
                if presentation.set_studio_preview_page(Some(output.page)) {
                    surface.0 = output.image.and_then(|image| {
                        Some(UiPreviewImage {
                            image,
                            page: u32::from(presentation.studio_preview_icon()?.page),
                            placeholder_identity: identity,
                        })
                    });
                    PreviewStatus {
                        ready: true,
                        cadence: Some(request.frame_rate()),
                        at: completed.at,
                        epoch: Some(request.epoch),
                        notice: output.notice,
                        camera: output.camera,
                        ..Default::default()
                    }
                } else {
                    PreviewStatus {
                        error: Some(
                            "The preview image exceeded the available UI texture budget.".into(),
                        ),
                        ..Default::default()
                    }
                }
            }
            Err(error) => PreviewStatus {
                error: Some(error),
                ..Default::default()
            },
        };
        if status.ready {
            state.displayed = Some(status.clone());
            state.displayed_identity = identity;
            state.displayed_generation = Some(generation);
        }
        menu.set_studio_preview_status(status);
    }
    if state.prepared.is_some() {
        return;
    }
    if !playing && state.has_frame(&request, due, generation) {
        state.requested = request.frame_rate().frame_at(due).ok();
        return;
    }
    let mut request = request;
    request.at = prepare_time(
        &request,
        state.displayed.as_ref(),
        state.displayed_generation == Some(generation),
        playing,
    )
    .unwrap_or(due);
    let number = request.frame_rate().frame_at(request.at).unwrap_or(0);
    let key = number;
    if state.requested == Some(key) {
        return;
    }
    let worker = state.worker.as_ref().expect("preview worker installed");
    if worker.submit(Job {
        generation,
        request,
        font,
        maximum,
    }) {
        state.requested = Some(key);
    }
}

/// Equal immutable font contents retain one owner so unrelated UI publication cannot cancel work.
fn retained_font(
    previous: Option<&Arc<assets::RuntimeFontCatalog>>,
    next: Arc<assets::RuntimeFontCatalog>,
) -> (Arc<assets::RuntimeFontCatalog>, bool) {
    if let Some(previous) = previous.filter(|previous| previous.identity() == next.identity()) {
        (Arc::clone(previous), false)
    } else {
        (next, true)
    }
}

/// A camera refresh coalesces behind its admitted frame; only a new history withdraws playback credit.
fn refresh_status(
    displayed: Option<&PreviewStatus>,
    epoch: [u64; 3],
    invalidate: bool,
) -> PreviewStatus {
    let mut status = displayed
        .filter(|status| status.epoch.is_some_and(|shown| shown[0] == epoch[0]))
        .cloned()
        .unwrap_or_default();
    let admitted =
        !invalidate && status.ready && status.epoch == Some(epoch) && status.error.is_none();
    status.ready = admitted;
    status.loading = !admitted;
    status.error = None;
    status
}

/// One future frame can render while the device consumes the current frame's existing playback credit.
fn prepare_time(
    request: &PreviewRequest,
    displayed: Option<&PreviewStatus>,
    admitted: bool,
    playing: bool,
) -> Result<recording_studio::MediaTime, String> {
    let due = request.frame_time()?;
    if playing
        && admitted
        && displayed.is_some_and(|status| {
            status.ready
                && status.epoch == Some(request.epoch)
                && status.at == due
                && status.error.is_none()
        })
    {
        let fps = request.frame_rate();
        let index = fps.frame_at(due).map_err(|error| error.to_string())?;
        if let Some(next) = index
            .checked_add(1)
            .and_then(|index| fps.frame_time(index).ok())
            .filter(|at| *at < request.project.duration())
        {
            return Ok(next);
        }
    }
    Ok(due)
}

#[derive(Debug, Eq, PartialEq)]
enum Delivery {
    Show,
    Hold,
    Discard,
}

/// A coalesced completion cannot move continuous playback back to an already superseded image.
fn follows_displayed(
    at: recording_studio::MediaTime,
    displayed: Option<recording_studio::MediaTime>,
    playing: bool,
) -> bool {
    !playing || displayed.is_none_or(|displayed| at >= displayed)
}

/// Speculative images keep their lease until due, and cannot change a paused view.
fn delivery(
    at: recording_studio::MediaTime,
    due: recording_studio::MediaTime,
    playing: bool,
) -> Delivery {
    if at <= due {
        Delivery::Show
    } else if playing {
        Delivery::Hold
    } else {
        Delivery::Discard
    }
}

impl Worker {
    /// A single replaceable pending request bounds drag/playback backlog without cancelling every forward frame.
    fn start(context: Option<ReplayRenderContext>) -> Result<Self, String> {
        let shared = Arc::new(Shared {
            pending: Mutex::new(None),
            changed: Condvar::new(),
            completed: Mutex::new(None),
            generation: AtomicU64::new(0),
            stopped: AtomicBool::new(false),
        });
        let worker = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("studio-preview".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run(&worker, context)
                }));
                if result.is_err() {
                    let generation = worker.generation.load(Ordering::Acquire);
                    *worker
                        .completed
                        .lock()
                        .unwrap_or_else(|error| error.into_inner()) = Some(Completed {
                        generation,
                        at: recording_studio::MediaTime::ZERO,
                        identity: None,
                        result: Err("The preview worker stopped unexpectedly.".into()),
                    });
                    worker.stopped.store(true, Ordering::Release);
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self { shared })
    }

    /// Identity changes cancel in-flight work; continuous forward requests coalesce behind it.
    fn submit(&self, job: Job) -> bool {
        if self.shared.stopped.load(Ordering::Acquire) {
            return false;
        }
        let Ok(mut pending) = self.shared.pending.try_lock() else {
            return false;
        };
        *pending = Some(job);
        self.shared.changed.notify_one();
        true
    }

    /// The frame owner never waits for a worker publishing its immutable result.
    fn poll(&self) -> Option<Completed> {
        self.shared.completed.try_lock().ok()?.take()
    }
}

impl Drop for Worker {
    /// Process cleanup happens on the worker. Closing the editor never joins a decoder on the input thread.
    fn drop(&mut self) {
        self.shared.stopped.store(true, Ordering::Release);
        self.shared.generation.fetch_add(1, Ordering::AcqRel);
        self.shared.changed.notify_one();
    }
}

/// All blocking waits, decoding, composition and page hashing are confined to this worker.
fn run(shared: &Shared, context: Option<ReplayRenderContext>) {
    let mut preview = RasterPreview::interactive();
    let mut composition = context
        .clone()
        .map(|context| super::composition::Composition::new(context, true));
    let mut titles = super::titles::Titles::new(context.clone());
    let mut native_page: Option<UiTexturePage> = None;
    let mut metrics = metrics::PreviewMetrics::default();
    loop {
        let job = {
            let mut pending = shared
                .pending
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            while pending.is_none() && !shared.stopped.load(Ordering::Acquire) {
                pending = shared
                    .changed
                    .wait_timeout(pending, std::time::Duration::from_millis(100))
                    .unwrap_or_else(|error| error.into_inner())
                    .0;
            }
            if shared.stopped.load(Ordering::Acquire) {
                return;
            }
            pending.take().expect("preview job signalled")
        };
        let cancelled = || {
            shared.stopped.load(Ordering::Acquire)
                || shared.generation.load(Ordering::Acquire) != job.generation
        };
        if cancelled() {
            continue;
        }
        let started = std::time::Instant::now();
        let wait = || {
            let pending = shared
                .pending
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let _ = shared
                .changed
                .wait_timeout(pending, std::time::Duration::from_millis(10));
        };
        let mut native_source = false;
        let maximum = job.maximum;
        titles.set_font(job.font);
        let result = if let Some(composition) = &mut composition {
            composition
                .render(&job.request, maximum, &cancelled, &wait)
                .and_then(|frame| {
                    native_source = frame.native;
                    let image = titles.render(&job.request, frame.image, &cancelled, &wait)?;
                    gpu_output(&mut native_page, image, frame.notice, frame.camera)
                })
        } else {
            preview
                .render(&job.request, maximum, &cancelled)
                .and_then(|frame| {
                    if !frame.effects.neutral() || titles.active(&job.request)? {
                        return Err(
                            "Timeline effects and titles need an initialized graphics device."
                                .into(),
                        );
                    }
                    UiTexturePage::owned(frame.size, frame.rgba8)
                        .map(|page| PreviewOutput {
                            page,
                            image: None,
                            notice: frame.notice,
                            camera: None,
                        })
                        .map_err(|error| format!("Preview image admission failed: {error:?}"))
                })
        };
        if cancelled() {
            continue;
        }
        metrics.frame(
            &job.request,
            started.elapsed(),
            native_source,
            composition.as_ref().map_or_else(
                || preview.decoder_counts(),
                super::composition::Composition::decoder_counts,
            ),
            result.as_ref().err().map(String::as_str),
        );
        *shared
            .completed
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(Completed {
            generation: job.generation,
            at: job.request.frame_time().unwrap_or(job.request.at),
            identity: Some(RequestIdentity::new(&job.request)),
            result,
        });
    }
}

/// All composed GPU frames share one placeholder per extent; per-frame pixels never pass through UI hashing.
fn gpu_output(
    page: &mut Option<UiTexturePage>,
    image: Arc<PreviewImage>,
    notice: Option<String>,
    camera: Option<CameraPreview>,
) -> Result<PreviewOutput, String> {
    let size = image.target.size();
    if page.as_ref().is_none_or(|page| page.dimensions() != size) {
        let bytes = recording_studio::video::frame_byte_len(size)?;
        *page = Some(
            UiTexturePage::owned(size, vec![0; bytes].into())
                .map_err(|error| format!("Composed preview page was rejected: {error:?}"))?,
        );
    }
    Ok(PreviewOutput {
        page: page.as_ref().expect("composed page admitted").clone(),
        image: Some(image),
        notice,
        camera,
    })
}
