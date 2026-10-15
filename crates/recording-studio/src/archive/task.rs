//! Nonblocking producer and observable worker lifecycle for live recording.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex, TryLockError,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, SyncSender, TrySendError},
    },
};

use super::{
    ArchiveError, Completion, RecordInput, RecordingConfig, RecordingInfo,
    format::RECORD_HEADER_BYTES, io, writer::Writer,
};
use crate::MediaTime;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordingState {
    Starting,
    Recording,
    Finishing,
    Complete,
    Interrupted,
    Failed,
}

#[derive(Clone, Debug)]
pub struct RecordingProgress {
    pub state: RecordingState,
    pub queued_bytes: usize,
    pub processed_records: u64,
    pub committed_records: u64,
    pub segments: u32,
    pub stored_bytes: u64,
    pub duration: MediaTime,
    pub error: Option<String>,
}

struct Shared {
    path: PathBuf,
    progress: Mutex<RecordingProgress>,
    changed: Condvar,
    queued_bytes: AtomicUsize,
    accepting: AtomicBool,
    failed: AtomicBool,
    finish: AtomicBool,
    stop_at: AtomicU64,
}

/// Polling a task never joins a thread, performs disk IO or waits for the worker's progress lock.
#[derive(Clone)]
pub struct RecordingTask(Arc<Shared>);

impl RecordingTask {
    pub fn path(&self) -> &Path {
        &self.0.path
    }

    /// Returns None only during a concurrent progress publication; the UI retains its last snapshot.
    pub fn progress(&self) -> Option<RecordingProgress> {
        let progress = match self.0.progress.try_lock() {
            Ok(progress) => progress,
            Err(TryLockError::WouldBlock) => return None,
            Err(TryLockError::Poisoned(error)) => error.into_inner(),
        };
        let mut snapshot = progress.clone();
        snapshot.queued_bytes = self.0.queued_bytes.load(Ordering::Acquire);
        Some(snapshot)
    }

    /// Waits for durable completion on a finalization worker, never on a game or UI thread.
    pub fn wait(&self) -> RecordingProgress {
        let mut progress = self
            .0
            .progress
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        while matches!(
            progress.state,
            RecordingState::Starting | RecordingState::Recording | RecordingState::Finishing
        ) {
            progress = self
                .0
                .changed
                .wait(progress)
                .unwrap_or_else(|error| error.into_inner());
        }
        progress.clone()
    }
}

/// One ordered producer owns the recording clock. Full queues stop recording rather than stalling play
/// or silently dropping world changes that would make subsequent replay frames untrustworthy.
pub struct Recording {
    sender: Option<SyncSender<QueuedRecord>>,
    task: RecordingTask,
    maximum_queued_bytes: usize,
    next_sequence: u64,
    last_at: MediaTime,
}

impl Recording {
    /// Starts storage asynchronously; callers inspect the task for directory/permission failures.
    pub fn start(
        path: PathBuf,
        info: RecordingInfo,
        config: RecordingConfig,
    ) -> Result<Self, ArchiveError> {
        Self::spawn(path, info, config, None)
    }

    /// Opens storage on the caller's IO worker before related video/audio files are created.
    /// The returned producer is nonblocking; this startup method must not run on the frame thread.
    pub fn start_ready(
        path: PathBuf,
        info: RecordingInfo,
        config: RecordingConfig,
    ) -> Result<Self, ArchiveError> {
        let writer = Writer::create(path.clone(), &info, config)?;
        Self::spawn(path, info, config, Some(writer))
    }

    /// Shares admission and lifecycle between asynchronous and already-open storage.
    fn spawn(
        path: PathBuf,
        info: RecordingInfo,
        config: RecordingConfig,
        ready: Option<Writer>,
    ) -> Result<Self, ArchiveError> {
        info.validate()?;
        config.validate()?;
        let shared = Arc::new(Shared {
            path: path.clone(),
            progress: Mutex::new(RecordingProgress {
                state: RecordingState::Starting,
                queued_bytes: 0,
                processed_records: 0,
                committed_records: 0,
                segments: 0,
                stored_bytes: 0,
                duration: MediaTime::ZERO,
                error: None,
            }),
            changed: Condvar::new(),
            queued_bytes: AtomicUsize::new(0),
            accepting: AtomicBool::new(true),
            failed: AtomicBool::new(false),
            finish: AtomicBool::new(false),
            stop_at: AtomicU64::new(0),
        });
        let (sender, receiver) = mpsc::sync_channel(config.queued_records);
        let worker = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("recording-storage".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run(&worker, info, config, receiver, ready)
                }));
                if result.is_err() {
                    worker.fail("recording storage worker stopped unexpectedly".into());
                }
            })
            .map_err(|error| io(&path, error))?;
        Ok(Self {
            sender: Some(sender),
            task: RecordingTask(shared),
            maximum_queued_bytes: config.queued_bytes,
            next_sequence: 1,
            last_at: MediaTime::ZERO,
        })
    }

    pub fn task(&self) -> RecordingTask {
        self.task.clone()
    }

    /// Transfers an Arc payload through a bounded queue, with no compression, serialization or IO here.
    pub fn push(&mut self, record: RecordInput) -> Result<u64, ArchiveError> {
        if let Err(error) = record.validate() {
            self.fail(&error);
            return Err(error);
        }
        let bytes = record.payload.len();
        self.enqueue(
            record.at,
            record.kind,
            Payload::Bytes(record.payload),
            bytes,
            false,
        )
    }

    /// Publishes a small ordered record from an IO worker, waiting only for a channel slot.
    /// Live frame producers must use `push` or `push_prepared`, which remain nonblocking.
    pub fn push_on_worker(&mut self, record: RecordInput) -> Result<u64, ArchiveError> {
        if let Err(error) = record.validate() {
            self.fail(&error);
            return Err(error);
        }
        let bytes = record.payload.len();
        self.enqueue(
            record.at,
            record.kind,
            Payload::Bytes(record.payload),
            bytes,
            true,
        )
    }

    /// Transfers immutable scene references and serializes them only on the existing storage worker.
    /// `retained_bytes` bounds the allocations held by the closure, including shared buffers it pins.
    pub fn push_prepared<F>(
        &mut self,
        at: MediaTime,
        kind: super::RecordKind,
        retained_bytes: usize,
        encode: F,
    ) -> Result<u64, ArchiveError>
    where
        F: FnOnce(&mut dyn std::io::Write) -> Result<(), ArchiveError> + Send + 'static,
    {
        let retained_bytes = retained_bytes.max(std::mem::size_of::<F>());
        self.enqueue(
            at,
            kind,
            Payload::Prepared(Box::new(encode)),
            retained_bytes,
            false,
        )
    }

    /// Admission reserves both queue slots and pinned heap bytes without touching the filesystem.
    fn enqueue(
        &mut self,
        at: MediaTime,
        kind: super::RecordKind,
        payload: Payload,
        retained_bytes: usize,
        wait_for_slot: bool,
    ) -> Result<u64, ArchiveError> {
        if !self.task.0.accepting.load(Ordering::Acquire) || self.sender.is_none() {
            self.sender.take();
            return Err(ArchiveError::Closed);
        }
        if at < self.last_at
            || at.0 > crate::time::MAX_MEDIA_MICROS
            || self.next_sequence == u64::MAX
        {
            let error = ArchiveError::Invalid(
                "recording timestamps moved backwards or sequence was exhausted",
            );
            self.fail(&error);
            return Err(error);
        }
        let Some(bytes) = retained_bytes.checked_add(RECORD_HEADER_BYTES) else {
            self.fail(&ArchiveError::Backpressure);
            return Err(ArchiveError::Backpressure);
        };
        if self
            .task
            .0
            .queued_bytes
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |queued| {
                queued
                    .checked_add(bytes)
                    .filter(|&value| value <= self.maximum_queued_bytes)
            })
            .is_err()
        {
            self.fail(&ArchiveError::Backpressure);
            return Err(ArchiveError::Backpressure);
        }
        let sequence = self.next_sequence;
        let queued = QueuedRecord {
            sequence,
            at,
            kind,
            payload,
            charge: QueueCharge {
                shared: Arc::clone(&self.task.0),
                bytes,
            },
        };
        let sender = self.sender.as_ref().ok_or(ArchiveError::Closed)?;
        let result = if wait_for_slot {
            sender
                .send(queued)
                .map_err(|error| TrySendError::Disconnected(error.0))
        } else {
            sender.try_send(queued)
        };
        match result {
            Ok(()) => {
                self.next_sequence += 1;
                self.last_at = at;
                Ok(sequence)
            }
            Err(error) => {
                let error = match error {
                    TrySendError::Full(_) => ArchiveError::Backpressure,
                    TrySendError::Disconnected(_) => ArchiveError::Closed,
                };
                self.fail(&error);
                Err(error)
            }
        }
    }

    /// Closes admission immediately, then lets the worker drain and finalize without blocking the UI.
    pub fn finish(mut self, at: MediaTime) -> Result<RecordingTask, ArchiveError> {
        if at < self.last_at || at.0 > crate::time::MAX_MEDIA_MICROS {
            let error = ArchiveError::Invalid("recording stop precedes its latest sample");
            self.fail(&error);
            return Err(error);
        }
        if self.task.0.failed.load(Ordering::Acquire) {
            return Err(ArchiveError::Closed);
        }
        self.task.0.stop_at.store(at.0, Ordering::Release);
        self.task.0.finish.store(true, Ordering::Release);
        self.task.0.accepting.store(false, Ordering::Release);
        self.task.0.publish(|progress| {
            if progress.state != RecordingState::Failed {
                progress.state = RecordingState::Finishing;
            }
        });
        self.sender.take();
        Ok(self.task.clone())
    }

    /// Publishes a producer failure once, then releases the queue endpoint so the worker can recover.
    fn fail(&mut self, error: &ArchiveError) {
        self.task.0.fail(error.to_string());
        self.sender.take();
    }
}

impl Drop for Recording {
    /// An abandoned handle leaves recoverable committed segments rather than blocking app shutdown.
    fn drop(&mut self) {
        self.task.0.accepting.store(false, Ordering::Release);
        self.sender.take();
    }
}

struct QueuedRecord {
    sequence: u64,
    at: MediaTime,
    kind: super::RecordKind,
    payload: Payload,
    charge: QueueCharge,
}

type Encode = Box<dyn FnOnce(&mut dyn std::io::Write) -> Result<(), ArchiveError> + Send>;

enum Payload {
    Bytes(Arc<[u8]>),
    Prepared(Encode),
}

impl Payload {
    /// A bounded writer rejects excessive serialization before it can allocate an oversized record.
    fn materialize(self) -> Result<Arc<[u8]>, ArchiveError> {
        match self {
            Self::Bytes(bytes) => Ok(bytes),
            Self::Prepared(encode) => {
                let mut out = BoundedBuffer(Vec::new());
                encode(&mut out)?;
                Ok(out.0.into())
            }
        }
    }
}

struct BoundedBuffer(Vec<u8>);

impl std::io::Write for BoundedBuffer {
    /// Bounds every write, including streaming serializers which do not know their final byte count.
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > super::MAX_RECORD_BYTES.saturating_sub(self.0.len()) {
            return Err(std::io::Error::other(
                "encoded replay record exceeds its byte limit",
            ));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct QueueCharge {
    shared: Arc<Shared>,
    bytes: usize,
}

impl Drop for QueueCharge {
    /// Releases reservations on consumption, failed sends, worker errors and unwinding alike.
    fn drop(&mut self) {
        self.shared
            .queued_bytes
            .fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

impl Shared {
    /// The worker only holds this lock while assigning small metadata, never during compression or IO.
    fn publish(&self, update: impl FnOnce(&mut RecordingProgress)) {
        let mut progress = self
            .progress
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        update(&mut progress);
        drop(progress);
        self.changed.notify_all();
    }

    /// Retains the original failure; secondary cleanup errors must not hide its actionable cause.
    fn fail(&self, message: String) {
        self.accepting.store(false, Ordering::Release);
        self.failed.store(true, Ordering::Release);
        self.publish(|progress| {
            progress.state = RecordingState::Failed;
            if progress.error.is_none() {
                progress.error = Some(message);
            }
        });
    }

    /// Reflects durable commits separately from accepted/processed records.
    fn committed(&self, bytes: u64, completion: Completion) {
        self.publish(|progress| {
            progress.committed_records = completion.records;
            progress.segments = completion.segments;
            progress.stored_bytes = bytes;
            progress.duration = completion.duration;
        });
    }
}

/// Owns all mutable filesystem state; terminating this worker never affects the live network session.
fn run(
    shared: &Shared,
    info: RecordingInfo,
    config: RecordingConfig,
    receiver: mpsc::Receiver<QueuedRecord>,
    ready: Option<Writer>,
) {
    let mut writer =
        match ready.map_or_else(|| Writer::create(shared.path.clone(), &info, config), Ok) {
            Ok(writer) => writer,
            Err(error) => {
                shared.fail(error.to_string());
                return;
            }
        };
    shared.publish(|progress| {
        if !matches!(
            progress.state,
            RecordingState::Failed | RecordingState::Finishing
        ) {
            progress.state = RecordingState::Recording;
        }
    });
    while let Ok(queued) = receiver.recv() {
        let QueuedRecord {
            sequence,
            at,
            kind,
            payload,
            charge,
        } = queued;
        drop(charge);
        if shared.failed.load(Ordering::Acquire) {
            break;
        }
        let payload = match payload.materialize() {
            Ok(payload) => payload,
            Err(error) => {
                shared.fail(error.to_string());
                return;
            }
        };
        let record = RecordInput { at, kind, payload };
        if let Err(error) = writer.append(sequence, &record) {
            shared.fail(error.to_string());
            return;
        }
        shared.publish(|progress| progress.processed_records += 1);
        let (bytes, completion) = writer.progress();
        shared.committed(bytes, completion);
    }
    // No producer remains after recv closes. A failed producer may still have queued records to drop.
    drop(receiver);
    let normal_stop =
        shared.finish.load(Ordering::Acquire) && !shared.failed.load(Ordering::Acquire);
    let result = if normal_stop {
        writer.complete(MediaTime(shared.stop_at.load(Ordering::Acquire)))
    } else {
        writer.interrupt()
    };
    shared.accepting.store(false, Ordering::Release);
    match result {
        Ok((bytes, completion)) => shared.publish(|progress| {
            progress.committed_records = completion.records;
            progress.segments = completion.segments;
            progress.stored_bytes = bytes;
            progress.duration = completion.duration;
            if progress.state != RecordingState::Failed {
                progress.state = if normal_stop {
                    RecordingState::Complete
                } else {
                    RecordingState::Interrupted
                };
            }
        }),
        Err(error) => shared.fail(error.to_string()),
    }
}
