//! One take-wide admission counter covers physical file growth and temporary publications.

use std::{
    fs::File,
    io::{self, Seek, SeekFrom, Write},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

/// The live take allowance includes journals, immutable assets, PCM, video and publication staging.
pub const DEFAULT_TAKE_BYTES: u64 = 25 * 1024 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct DiskBudget(Arc<State>);
#[derive(Debug)]
struct State {
    maximum: u64,
    used: AtomicU64,
}

impl DiskBudget {
    /// The capture owner creates one budget and clones it into every worker belonging to that take.
    pub fn new(maximum: u64) -> Self {
        Self(Arc::new(State {
            maximum,
            used: AtomicU64::new(0),
        }))
    }
    /// Includes admitted writes that are in flight as well as bytes already retained on disk.
    pub fn used(&self) -> u64 {
        self.0.used.load(Ordering::Acquire)
    }
    /// All writers use the same immutable aggregate allowance.
    pub fn maximum(&self) -> u64 {
        self.0.maximum
    }
    /// Existing immutable stores are adopted once by their sole storage owner, on its IO worker.
    pub fn adopt(&self, bytes: u64) -> io::Result<()> {
        self.reserve(bytes)?.commit(bytes);
        Ok(())
    }
    /// Bytes are refunded only after truncation or removal has physically succeeded.
    fn release(&self, bytes: u64) {
        if bytes != 0 {
            self.0
                .used
                .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                    used.checked_sub(bytes)
                })
                .expect("a physical refund must belong to this take's admission owner");
        }
    }
    /// Checked nonblocking admission is independent of filesystem calls and other stream workers.
    fn reserve(&self, bytes: u64) -> io::Result<Reservation> {
        self.0.used.try_update(Ordering::AcqRel, Ordering::Acquire, |used| used.checked_add(bytes).filter(|&total| total <= self.maximum()))
            .map_err(|used| io::Error::other(format!("Take-wide recording disk limit reached: {used} bytes admitted, {bytes} additional bytes requested, {} byte limit. Committed recording prefixes remain recoverable.", self.maximum())))?;
        Ok(Reservation {
            budget: self.clone(),
            bytes,
        })
    }
}

struct Reservation {
    budget: DiskBudget,
    bytes: u64,
}
impl Reservation {
    /// Only successfully written growth becomes persistent admission; failed or short writes return the rest.
    fn commit(mut self, bytes: u64) {
        self.budget.release(
            self.bytes
                .checked_sub(bytes)
                .expect("a write cannot grow beyond its admitted extent"),
        );
        self.bytes = 0;
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        self.budget.release(self.bytes);
    }
}

/// This handle's extent belongs to exactly one physical file, including after it is closed or renamed.
#[derive(Debug)]
pub struct Metered<W> {
    inner: W,
    budget: DiskBudget,
    extent: Arc<AtomicU64>,
    position: u64,
}

impl Metered<File> {
    /// Adopts one exclusively owned, non-append file; no independent writer may mutate its extent.
    pub fn file(mut file: File, budget: DiskBudget) -> io::Result<Self> {
        let length = file.metadata()?.len();
        let position = file.stream_position()?;
        budget.adopt(length)?;
        Ok(Self {
            inner: file,
            budget,
            extent: Arc::new(AtomicU64::new(length)),
            position,
        })
    }
    /// Header rewrites do not grow a file; shrinking returns bytes only after the OS confirms truncation.
    pub fn set_len(&mut self, length: u64) -> io::Result<()> {
        let old = self.extent.load(Ordering::Acquire);
        let growth = length.saturating_sub(old);
        let reservation = self.budget.reserve(growth)?;
        self.inner.set_len(length)?;
        self.extent.store(length, Ordering::Release);
        reservation.commit(growth);
        self.budget.release(old.saturating_sub(length));
        Ok(())
    }
    /// Durable completion changes no admission count because the file still occupies its extent.
    pub fn sync_all(&self) -> io::Result<()> {
        self.inner.sync_all()
    }
    /// Index commits may flush data without republishing the file's directory entry.
    pub fn sync_data(&self) -> io::Result<()> {
        self.inner.sync_data()
    }
    /// Manifest validation can inspect the same owned file without reopening a path.
    pub fn metadata(&self) -> io::Result<std::fs::Metadata> {
        self.inner.metadata()
    }
}

impl<W> Metered<W> {
    /// Moving the underlying handle preserves its physical charge until explicit deletion/truncation.
    pub fn into_inner(self) -> W {
        self.inner
    }
}

impl<W: Write> Write for Metered<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let end = self
            .position
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| io::Error::other("Recording file extent overflowed."))?;
        let old = self.extent.load(Ordering::Acquire);
        let reservation = self.budget.reserve(end.saturating_sub(old))?;
        let count = self.inner.write(bytes)?;
        self.position += count as u64;
        let growth = self.position.saturating_sub(old);
        self.extent.store(old.max(self.position), Ordering::Release);
        reservation.commit(growth);
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
impl<W: Seek> Seek for Metered<W> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let at = self.inner.seek(position)?;
        self.position = at;
        Ok(at)
    }
}

/// Staging is charged while it physically exists, including during hash verification and deduplication.
#[derive(Debug)]
pub struct StagedFile {
    file: Option<tempfile::NamedTempFile>,
    budget: DiskBudget,
    extent: Arc<AtomicU64>,
}
#[derive(Debug)]
pub struct PersistError {
    pub error: io::Error,
    pub file: StagedFile,
}
impl StagedFile {
    /// Allocates an empty staging file on a storage worker; no destination is overwritten.
    pub fn new(parent: &Path, prefix: &str, suffix: &str, budget: DiskBudget) -> io::Result<Self> {
        let file = tempfile::Builder::new()
            .prefix(prefix)
            .suffix(suffix)
            .tempfile_in(parent)?;
        Ok(Self {
            file: Some(file),
            budget,
            extent: Arc::new(AtomicU64::new(0)),
        })
    }
    /// A mutable borrow prevents concurrent metered writers to the same physical staging file.
    pub fn writer(&mut self) -> io::Result<Metered<&mut File>> {
        let inner = self
            .file
            .as_mut()
            .expect("owned staging file")
            .as_file_mut();
        let position = inner.stream_position()?;
        Ok(Metered {
            inner,
            budget: self.budget.clone(),
            extent: Arc::clone(&self.extent),
            position,
        })
    }
    /// Syncing remains on the same worker that owns publication and cleanup.
    pub fn sync_all(&self) -> io::Result<()> {
        self.file
            .as_ref()
            .expect("owned staging file")
            .as_file()
            .sync_all()
    }
    /// A successful rename retains its charge; collision/error returns ownership for physical cleanup.
    pub fn persist_noclobber(mut self, path: &Path) -> Result<File, PersistError> {
        match self
            .file
            .take()
            .expect("owned staging file")
            .persist_noclobber(path)
        {
            Ok(file) => Ok(file),
            Err(error) => {
                self.file = Some(error.file);
                Err(PersistError {
                    error: error.error,
                    file: self,
                })
            }
        }
    }
    /// The replaced video must be exclusively owned and already charged to this same take.
    /// Its bytes are refunded only after the directory entry has physically been replaced.
    pub(crate) fn persist_replace(mut self, path: &Path) -> Result<File, PersistError> {
        let old = match std::fs::metadata(path) {
            Ok(metadata) => metadata.len(),
            Err(error) => return Err(PersistError { error, file: self }),
        };
        match self.file.take().expect("owned staging file").persist(path) {
            Ok(file) => {
                self.budget.release(old);
                Ok(file)
            }
            Err(error) => {
                self.file = Some(error.file);
                Err(PersistError {
                    error: error.error,
                    file: self,
                })
            }
        }
    }
    /// Completed companions survive cancellation/publication failure at an explicit recovery path.
    pub fn retain(mut self, mut error: String) -> String {
        let path = match self.file.take().expect("owned staging file").keep() {
            Ok((_, path)) => path,
            Err(keep) => {
                error.push_str(&format!(
                    " Cannot mark the completed file permanent: {}.",
                    keep.error
                ));
                let mut path = keep.file.into_temp_path();
                path.disable_cleanup(true);
                path.to_path_buf()
            }
        };
        format!("{error} The completed media remains at {}.", path.display())
    }
}

/// Directory durability follows file synchronization and atomic publication on its storage worker.
pub(crate) fn sync_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}
impl Drop for StagedFile {
    fn drop(&mut self) {
        if let Some(file) = self.file.take()
            && file.close().is_ok()
        {
            self.budget.release(self.extent.load(Ordering::Acquire));
        }
    }
}

#[cfg(test)]
mod tests;
