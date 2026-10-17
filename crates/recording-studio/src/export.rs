//! Exact-frame export runs on one owned worker and publishes only a completely encoded file.

use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    },
};

use crate::{
    MediaTime, Project,
    preview::PreviewRequest,
    video::{EncodeOptions, EncodingSpeed, VideoSettings, VideoWriter, frame_byte_len},
};

mod audio;
mod files;

const VIDEO_BYTE_LIMIT: u64 = 64 * 1024 * 1024 * 1024;
const RUNNING: u8 = 0;
const CANCELLED: u8 = 1;
const PUBLISHING: u8 = 2;

#[derive(Clone)]
pub struct Cancellation(Arc<AtomicU8>);

impl Default for Cancellation {
    /// A new job always starts in the same state used by cancellation/publication transitions.
    fn default() -> Self {
        Self(Arc::new(AtomicU8::new(RUNNING)))
    }
}

impl Cancellation {
    /// Cancellation is accepted until completed-file publication begins; this never performs IO.
    pub fn cancel(&self) -> bool {
        self.0
            .compare_exchange(RUNNING, CANCELLED, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    /// Publication already in progress is terminal work, not an accepted cancellation.
    pub fn cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire) == CANCELLED
    }

    /// The same token reaches render, decode, audio mixing and supervised process IO.
    pub fn check(&self) -> Result<(), String> {
        if self.cancelled() {
            Err("Export cancelled.".into())
        } else {
            Ok(())
        }
    }

    /// The final publication is indivisible from cancellation's point of view.
    fn begin_publication(&self) -> Result<(), String> {
        self.0
            .compare_exchange(RUNNING, PUBLISHING, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|_| "Export cancelled.".into())
    }
}

#[derive(Clone)]
pub struct ExportRequest {
    pub project: Arc<Project>,
    pub source_base: PathBuf,
    /// A chosen file is never overwritten. A directory generates a unique `Export N.mp4` name.
    pub destination: Destination,
}

#[derive(Clone, Debug)]
pub enum Destination {
    File(PathBuf),
    Directory(PathBuf),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ExportPhase {
    #[default]
    Idle,
    Preparing,
    Rendering,
    FinishingVideo,
    MixingAudio,
    Muxing,
    Publishing,
    Complete,
    Cancelled,
    Failed,
}

impl ExportPhase {
    /// Terminal states remain visible after a worker exits; leaving Studio does not lose the result.
    pub const fn busy(self) -> bool {
        matches!(
            self,
            Self::Preparing
                | Self::Rendering
                | Self::FinishingVideo
                | Self::MixingAudio
                | Self::Muxing
                | Self::Publishing
        )
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExportStatus {
    pub phase: ExportPhase,
    pub frames: u64,
    pub total_frames: u64,
    pub encoder: Option<crate::model::Encoder>,
    pub path: Option<PathBuf>,
    pub error: Option<String>,
    pub notice: Option<String>,
}

pub struct ExportFrame {
    pub size: [u32; 2],
    pub rgba8: Arc<[u8]>,
    pub notice: Option<String>,
}

/// A render edge creates its GPU context inside the export worker, which need not itself be `Send`.
pub trait FrameSource {
    /// Admits the complete requested edit before the encoder is started; payload IO may remain incremental.
    fn prepare(
        &mut self,
        request: &ExportRequest,
        cancellation: &Cancellation,
    ) -> Result<(), String>;
    /// Returns one complete output-sized SDR frame at the requested timeline timestamp.
    fn frame(
        &mut self,
        request: &PreviewRequest,
        cancellation: &Cancellation,
    ) -> Result<ExportFrame, String>;
}

pub struct ExportTask {
    cancellation: Cancellation,
    status: Arc<Mutex<ExportStatus>>,
}

impl ExportTask {
    /// Only thread startup happens here; admission, rendering, storage and child IO remain off the UI thread.
    pub fn start<S: FrameSource + 'static>(
        request: ExportRequest,
        factory: impl FnOnce() -> Result<S, String> + Send + 'static,
    ) -> Result<Self, String> {
        let cancellation = Cancellation::default();
        let status = Arc::new(Mutex::new(ExportStatus {
            phase: ExportPhase::Preparing,
            ..Default::default()
        }));
        let worker_cancel = cancellation.clone();
        let worker_status = Arc::clone(&status);
        std::thread::Builder::new()
            .name("studio-export".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run(request, factory, &worker_cancel, &worker_status)
                }))
                .unwrap_or_else(|_| {
                    Err(
                        "The export worker panicked. Check the output directory before retrying."
                            .into(),
                    )
                });
                if let Err(error) = result {
                    update(&worker_status, |status| {
                        status.phase = if worker_cancel.cancelled() {
                            ExportPhase::Cancelled
                        } else {
                            ExportPhase::Failed
                        };
                        status.error = (!worker_cancel.cancelled()).then_some(error);
                    });
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            cancellation,
            status,
        })
    }

    /// Compact metadata is the only data shared back to the UI.
    pub fn status(&self) -> ExportStatus {
        self.status
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    /// Returns false once atomic finished-file publication has begun.
    pub fn cancel(&self) -> bool {
        self.cancellation.cancel()
    }
}

impl Drop for ExportTask {
    /// No join, device wait or subprocess kill takes place on the input thread.
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

/// Every frame is evaluated at an integer rational timestamp; real-time preview speed cannot drop output frames.
fn run<S: FrameSource>(
    mut request: ExportRequest,
    factory: impl FnOnce() -> Result<S, String>,
    cancellation: &Cancellation,
    status: &Mutex<ExportStatus>,
) -> Result<(), String> {
    request
        .project
        .validate()
        .map_err(|error| error.to_string())?;
    cancellation.check()?;
    if request.source_base.is_relative() {
        request.source_base = std::env::current_dir()
            .map_err(|error| error.to_string())?
            .join(&request.source_base);
    }
    let project = &request.project;
    if project.duration() == MediaTime::ZERO {
        return Err("The project has no duration to export.".into());
    }
    frame_byte_len(project.export.size)?;
    let total = project
        .export
        .fps
        .frames_covering(project.duration())
        .map_err(|error| error.to_string())?;
    let duration = project
        .export
        .fps
        .frame_time(total)
        .map_err(|error| error.to_string())?;
    let with_audio = project.export.include_audio
        && project
            .audio
            .iter()
            .any(|clip| !clip.muted && clip.gain > 0.0);
    if with_audio {
        audio::byte_length(duration)?;
    }
    let parent = files::parent(&request.destination)?;
    let destination = match &request.destination {
        Destination::Directory(_) => Destination::Directory(parent.clone()),
        Destination::File(path) => {
            Destination::File(parent.join(path.file_name().ok_or("Export filename is missing.")?))
        }
    };
    if let Destination::File(path) = &destination {
        match fs::symlink_metadata(path) {
            Ok(_) => {
                return Err(
                    "The chosen export file already exists. Choose a different filename.".into(),
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    fs::create_dir_all(&parent).map_err(|error| error.to_string())?;
    let scratch = tempfile::Builder::new()
        .prefix(".cinnabar-export-")
        .tempdir_in(&parent)
        .map_err(|error| error.to_string())?;
    let mut staged = files::staging(&parent)?;
    let mut source = factory()?;
    source.prepare(&request, cancellation)?;
    let settings = VideoSettings {
        path: staged.to_path_buf(),
        fps: project.export.fps,
        codec: project.export.codec,
    };
    let options = EncodeOptions {
        encoder: project.export.encoder,
        quality: Some(project.export.quality),
        speed: EncodingSpeed::Export,
    };
    let mut video = VideoWriter::start(
        &settings,
        project.export.size,
        options,
        cancellation.clone(),
    )?;
    update(status, |status| {
        status.phase = ExportPhase::Rendering;
        status.total_frames = total;
        status.encoder = Some(video.encoder());
    });
    let mut preview = PreviewRequest {
        epoch: [0; 3],
        project: Arc::clone(project),
        source_base: request.source_base.clone(),
        at: MediaTime::ZERO,
    };
    for index in 0..total {
        cancellation.check()?;
        preview.at = project
            .export
            .fps
            .frame_time(index)
            .map_err(|error| error.to_string())?;
        let frame = source.frame(&preview, cancellation)?;
        video.frame(index, frame.size, &frame.rgba8)?;
        update(status, |status| {
            status.frames = index + 1;
            if frame.notice.is_some() {
                status.notice = frame.notice;
            }
        });
        if index % 64 == 0 {
            files::check_bytes(&staged, VIDEO_BYTE_LIMIT)?;
        }
    }
    drop(source);
    update(status, |status| status.phase = ExportPhase::FinishingVideo);
    video.finish(total)?;
    files::check_bytes(&staged, VIDEO_BYTE_LIMIT)?;
    cancellation.check()?;
    if with_audio {
        update(status, |status| status.phase = ExportPhase::MixingAudio);
        let mix = audio::mix(&request, duration, scratch.path(), cancellation)?;
        update(status, |status| status.phase = ExportPhase::Muxing);
        let muxed = files::staging(&parent)?;
        audio::mux(&staged, &mix, &muxed, cancellation)?;
        files::check_bytes(&muxed, VIDEO_BYTE_LIMIT)?;
        staged = muxed;
    }
    crate::media_process::sync_output(&staged)?;
    cancellation.begin_publication()?;
    update(status, |status| status.phase = ExportPhase::Publishing);
    let path = files::publish(staged, &destination)?;
    update(status, |status| {
        status.phase = ExportPhase::Complete;
        status.path = Some(path);
    });
    Ok(())
}

/// Poisoned UI observation cannot prevent the worker from reporting its terminal result.
fn update(status: &Mutex<ExportStatus>, change: impl FnOnce(&mut ExportStatus)) {
    change(&mut status.lock().unwrap_or_else(|error| error.into_inner()));
}

/// Source references retain the same meaning during unsaved exports and saved-project exports.
fn source_path(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}
