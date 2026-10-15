//! Native editor preview bridge. Frame preparation, process IO and pixel hashing stay on one worker.

use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

use bevy::prelude::*;
use client_shell::{menu::MenuRuntime, ui_resources::UiPresentationResource};
use recording_studio::preview::{PreviewRequest, PreviewStatus, RasterPreview};
use render_model::{UI_STUDIO_PREVIEW_MAX_SIZE, UiTexturePage};

#[derive(Resource, Default)]
pub(super) struct NativePreview {
    worker: Option<Worker>,
    epoch: Option<[u64; 3]>,
    requested: Option<([u64; 3], u64)>,
    failed: Option<[u64; 3]>,
}

struct Job {
    generation: u64,
    request: PreviewRequest,
}
struct Completed {
    generation: u64,
    at: recording_studio::MediaTime,
    result: Result<(UiTexturePage, Option<String>), String>,
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
) {
    let Some(request) = menu.studio_preview_request() else {
        if state.worker.is_some() || state.epoch.is_some() || state.failed.is_some() {
            state.worker.take();
            state.epoch = None;
            state.requested = None;
            state.failed = None;
            presentation.set_studio_preview_page(None);
            menu.set_studio_preview_status(PreviewStatus::default());
        }
        return;
    };
    if state.failed == Some(request.epoch) {
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
        state.failed = Some(request.epoch);
        menu.set_studio_preview_status(PreviewStatus {
            error: Some(error),
            ..Default::default()
        });
        return;
    }
    if state.worker.is_none() {
        match Worker::start() {
            Ok(worker) => state.worker = Some(worker),
            Err(error) => {
                state.failed = Some(request.epoch);
                menu.set_studio_preview_status(PreviewStatus {
                    error: Some(error),
                    ..Default::default()
                });
                return;
            }
        }
    }
    let changed_epoch = state.epoch != Some(request.epoch);
    if changed_epoch {
        state.epoch = Some(request.epoch);
        state.requested = None;
        if let Some(worker) = &state.worker {
            worker.shared.generation.fetch_add(1, Ordering::AcqRel);
        }
        menu.set_studio_preview_status(PreviewStatus {
            loading: true,
            ..Default::default()
        });
    }
    let worker = state.worker.as_ref().expect("preview worker installed");
    let generation = worker.shared.generation.load(Ordering::Acquire);
    if let Some(completed) = worker
        .poll()
        .filter(|completed| completed.generation == generation)
    {
        let status = match completed.result {
            Ok((page, notice)) => {
                if presentation.set_studio_preview_page(Some(page)) {
                    PreviewStatus {
                        ready: true,
                        at: completed.at,
                        notice,
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
        menu.set_studio_preview_status(status);
    }
    let number = request.project.export.fps.frame_at(request.at).unwrap_or(0);
    let key = (request.epoch, number);
    if state.requested == Some(key) {
        return;
    }
    let worker = state.worker.as_ref().expect("preview worker installed");
    if worker.submit(Job {
        generation,
        request,
    }) {
        state.requested = Some(key);
    }
}

impl Worker {
    /// A single replaceable pending request bounds drag/playback backlog without cancelling every forward frame.
    fn start() -> Result<Self, String> {
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
                let result =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&worker)));
                if result.is_err() {
                    let generation = worker.generation.load(Ordering::Acquire);
                    *worker
                        .completed
                        .lock()
                        .unwrap_or_else(|error| error.into_inner()) = Some(Completed {
                        generation,
                        at: recording_studio::MediaTime::ZERO,
                        result: Err("The preview worker stopped unexpectedly.".into()),
                    });
                    worker.stopped.store(true, Ordering::Release);
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self { shared })
    }

    /// Only epoch changes cancel in-flight work; continuous forward requests coalesce behind it.
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
fn run(shared: &Shared) {
    let mut preview = RasterPreview::default();
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
        let result = preview
            .render(&job.request, UI_STUDIO_PREVIEW_MAX_SIZE, &cancelled)
            .and_then(|frame| {
                UiTexturePage::owned(frame.size, frame.rgba8)
                    .map(|page| (page, frame.notice))
                    .map_err(|error| format!("Preview image admission failed: {error:?}"))
            });
        if cancelled() {
            continue;
        }
        *shared
            .completed
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(Completed {
            generation: job.generation,
            at: job.request.at,
            result,
        });
    }
}
