//! Nonblocking native scene admission. Only the scene worker encodes meshes or touches the disk.

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};

use crossbeam_channel::{self as mpsc, Receiver, Sender as SyncSender, TryRecvError};

use assets::RuntimeAssets;
use recording_studio::{
    MediaTime,
    archive::{RecordingConfig, RecordingInfo, RecordingState},
    object_store::MAX_ASSET_BYTES,
};

use crate::{
    journal::{JournalPalette, TerrainJournal},
    palette::FrozenTint,
    scene::{ChunkId, MAX_SCENE_CHUNKS},
    terrain::TerrainMesh,
};

const QUEUED_BATCHES: usize = 256;
const MAX_PINNED_BYTES: usize = MAX_ASSET_BYTES as usize + 512 * 1024 * 1024;
const JOURNAL_DISK_BYTES: u64 = 64 * 1024 * 1024;
const CHECKPOINT_MICROS: u64 = 5_000_000;

struct Shared {
    pinned: AtomicUsize,
    cancelled: AtomicBool,
    stop: AtomicU64,
}

struct Charge {
    shared: Arc<Shared>,
    bytes: usize,
}
impl Drop for Charge {
    fn drop(&mut self) {
        self.shared.pinned.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

pub struct PreparedPalette {
    pub assets: Arc<RuntimeAssets>,
    pub tints: Arc<[FrozenTint]>,
    pub stream: u64,
    pub revision: u64,
    _charge: Charge,
}

pub struct PreparedChunk {
    pub key: ChunkId,
    pub generation: u64,
    pub mesh: TerrainMesh,
    pub palette: Arc<PreparedPalette>,
}

pub enum SceneChange {
    Upsert(PreparedChunk),
    Remove(ChunkId),
}

struct Batch {
    at: MediaTime,
    baseline: bool,
    changes: Vec<SceneChange>,
    _charge: Charge,
}

pub struct TerrainCapture {
    sender: Option<SyncSender<Batch>>,
    result: Receiver<Result<(), String>>,
    shared: Arc<Shared>,
    terminal: Option<Result<(), String>>,
    baseline_sent: bool,
    finishing: bool,
}

impl TerrainCapture {
    /// Starts an IO worker immediately; directory creation and large asset encoding never run here.
    pub fn start(parent: PathBuf, info: RecordingInfo) -> Result<Self, String> {
        let shared = Arc::new(Shared {
            pinned: AtomicUsize::new(0),
            cancelled: AtomicBool::new(false),
            stop: AtomicU64::new(0),
        });
        let (sender, jobs) = mpsc::bounded(QUEUED_BATCHES);
        let (completed, result) = mpsc::bounded(1);
        let worker_shared = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("studio-scene".into())
            .spawn(move || {
                let outcome = run(parent, info, jobs, &worker_shared);
                let _ = completed.send(outcome);
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            sender: Some(sender),
            result,
            shared,
            terminal: None,
            baseline_sent: false,
            finishing: false,
        })
    }

    /// Charges each immutable palette once for its full lifetime, even when many queued chunks share it.
    /// The caller creates another palette only when the actual texture or tint allocation changes.
    pub fn palette(
        &self,
        assets: Arc<RuntimeAssets>,
        tints: Arc<[FrozenTint]>,
        stream: u64,
        revision: u64,
    ) -> Result<Arc<PreparedPalette>, String> {
        let bytes = assets
            .recording_retained_bytes()
            .checked_add(std::mem::size_of_val(&*tints))
            .and_then(|bytes| bytes.checked_add(std::mem::size_of::<PreparedPalette>()))
            .ok_or("Recording palette byte count overflowed.")?;
        let charge = reserve(&self.shared, bytes)?;
        Ok(Arc::new(PreparedPalette {
            assets,
            tints,
            stream,
            revision,
            _charge: charge,
        }))
    }

    /// One coherent prepared snapshot starts at zero. Its buffers are shared with the native renderer.
    pub fn baseline(&mut self, chunks: Vec<SceneChange>) -> Result<(), String> {
        if self.baseline_sent {
            return Err("The scene baseline was already captured.".into());
        }
        self.submit(MediaTime::ZERO, true, chunks)?;
        self.baseline_sent = true;
        Ok(())
    }

    /// Each accepted delta is ordered and bounded; a slow disk stops capture rather than stalling play.
    pub fn changes(&mut self, at: MediaTime, changes: Vec<SceneChange>) -> Result<(), String> {
        if !self.baseline_sent {
            return Err("The recorded scene has no baseline.".into());
        }
        if changes.is_empty() {
            return Ok(());
        }
        self.submit(at, false, changes)
    }

    /// Polls completion without a join or lock; the native UI keeps a readable failure explanation.
    pub fn error(&mut self) -> Option<String> {
        if self.terminal.is_none() {
            match self.result.try_recv() {
                Ok(result) => self.terminal = Some(result),
                Err(TryRecvError::Disconnected) => {
                    self.terminal =
                        Some(Err("The recorded-scene worker stopped unexpectedly.".into()))
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        self.terminal
            .as_ref()
            .and_then(|result| result.as_ref().err())
            .cloned()
    }

    /// Called only by the capture finalizer. Closing the channel drains accepted changes first.
    pub fn finish(mut self, duration: MediaTime) -> Result<(), String> {
        self.shared.stop.store(duration.0, Ordering::Release);
        self.finishing = true;
        self.sender.take();
        self.terminal.take().unwrap_or_else(|| {
            self.result.recv().unwrap_or_else(|_| {
                Err("The recorded-scene finalizer stopped unexpectedly.".into())
            })
        })
    }

    /// Charges geometry before transferring its immutable references through the bounded queue.
    fn submit(
        &mut self,
        at: MediaTime,
        baseline: bool,
        changes: Vec<SceneChange>,
    ) -> Result<(), String> {
        if changes.len() > MAX_SCENE_CHUNKS {
            return Err("A recorded scene batch exceeds its chunk limit.".into());
        }
        let mut bytes = changes
            .capacity()
            .checked_mul(std::mem::size_of::<SceneChange>())
            .ok_or("Scene batch size overflowed.")?;
        for change in &changes {
            if let SceneChange::Upsert(chunk) = change {
                bytes = bytes
                    .checked_add(chunk.mesh.retained_bytes())
                    .ok_or("Scene mesh size overflowed.")?;
            }
        }
        let charge = reserve(&self.shared, bytes)?;
        self.sender
            .as_ref()
            .ok_or("Scene recording is finishing.")?
            .try_send(Batch {
                at,
                baseline,
                changes,
                _charge: charge,
            })
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => {
                    "The recorded-scene queue is full; the disk cannot keep up.".to_owned()
                }
                mpsc::TrySendError::Disconnected(_) => {
                    "The recorded-scene worker is unavailable.".to_owned()
                }
            })
    }
}

impl Drop for TerrainCapture {
    fn drop(&mut self) {
        if !self.finishing {
            self.shared.cancelled.store(true, Ordering::Release);
        }
        self.sender.take();
    }
}

/// Byte admission is atomic, bounded and released automatically on send failure or worker completion.
fn reserve(shared: &Arc<Shared>, bytes: usize) -> Result<Charge, String> {
    shared.pinned.fetch_update(Ordering::AcqRel, Ordering::Acquire, |old| old.checked_add(bytes).filter(|&total| total <= MAX_PINNED_BYTES))
        .map_err(|_| "Recorded scene buffers exceeded their memory budget; stopping instead of stalling gameplay.".to_owned())?;
    Ok(Charge {
        shared: Arc::clone(shared),
        bytes,
    })
}

/// Freezes only changed buffers and emits periodic reference-only checkpoints for efficient seeks.
fn run(
    parent: PathBuf,
    info: RecordingInfo,
    jobs: Receiver<Batch>,
    shared: &Shared,
) -> Result<(), String> {
    let defaults = RecordingConfig::default();
    let mut journal = TerrainJournal::start(
        &parent,
        info,
        RecordingConfig {
            disk_quota: JOURNAL_DISK_BYTES,
            ..defaults
        },
        defaults.disk_quota.saturating_sub(JOURNAL_DISK_BYTES),
    )
    .map_err(|error| error.to_string())?;
    let mut baseline = false;
    let mut palette: Option<(Arc<PreparedPalette>, JournalPalette)> = None;
    let mut checkpoint = MediaTime::ZERO;
    loop {
        if let Some(error) = journal.error() {
            return Err(error);
        }
        if shared.cancelled.load(Ordering::Acquire) {
            return Err("Scene recording was interrupted.".into());
        }
        let batch = match jobs.recv_timeout(Duration::from_millis(100)) {
            Ok(batch) => batch,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        if batch.baseline {
            if baseline || batch.at != MediaTime::ZERO {
                return Err("Invalid recorded-scene baseline order.".into());
            }
            journal
                .begin_baseline()
                .map_err(|error| error.to_string())?;
        } else if !baseline {
            return Err("Scene delta arrived before its baseline.".into());
        }
        for change in &batch.changes {
            if shared.cancelled.load(Ordering::Acquire) {
                return Err("Scene recording was interrupted.".into());
            }
            match change {
                SceneChange::Upsert(chunk) => {
                    if palette
                        .as_ref()
                        .is_none_or(|(current, _)| !Arc::ptr_eq(current, &chunk.palette))
                    {
                        let prepared = &chunk.palette;
                        let frozen = journal
                            .palette(
                                &prepared.assets,
                                &prepared.tints,
                                prepared.stream,
                                prepared.revision,
                            )
                            .map_err(|error| error.to_string())?;
                        palette = Some((Arc::clone(prepared), frozen));
                    }
                    journal
                        .upsert(
                            batch.at,
                            chunk.key,
                            chunk.generation,
                            &chunk.mesh,
                            &palette.as_ref().expect("prepared palette").1,
                        )
                        .map_err(|error| error.to_string())?;
                }
                SceneChange::Remove(key) if !batch.baseline => journal
                    .remove(batch.at, *key)
                    .map_err(|error| error.to_string())?,
                SceneChange::Remove(_) => {
                    return Err("A scene baseline cannot contain an unload.".into());
                }
            }
        }
        if batch.baseline {
            journal.end_baseline().map_err(|error| error.to_string())?;
            baseline = true;
        }
        if batch.at.0.saturating_sub(checkpoint.0) >= CHECKPOINT_MICROS {
            journal
                .checkpoint(batch.at)
                .map_err(|error| error.to_string())?;
            checkpoint = batch.at;
        }
    }
    if shared.cancelled.load(Ordering::Acquire) || !baseline {
        return Err("Scene recording ended before its baseline completed.".into());
    }
    let progress = journal
        .finish(MediaTime(shared.stop.load(Ordering::Acquire)))
        .map_err(|error| error.to_string())?
        .wait();
    if progress.state != RecordingState::Complete {
        return Err(progress
            .error
            .unwrap_or_else(|| "Recorded scene storage did not complete.".into()));
    }
    Ok(())
}
