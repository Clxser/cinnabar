use super::*;
use std::{io::Cursor, sync::Barrier};

/// Synthetic writers expose short writes and IO failures without relying on a filesystem race.
fn cursor(budget: &DiskBudget) -> Metered<Cursor<Vec<u8>>> {
    Metered {
        inner: Cursor::new(Vec::new()),
        budget: budget.clone(),
        extent: Arc::new(AtomicU64::new(0)),
        position: 0,
    }
}

#[test]
fn independent_streams_share_one_allowance_and_closed_files_remain_charged() {
    let temporary = tempfile::tempdir().unwrap();
    let budget = DiskBudget::new(12);
    for (name, contents) in [("scene", &[1; 5][..]), ("audio", &[2; 7][..])] {
        let file = File::create(temporary.path().join(name)).unwrap();
        let mut file = Metered::file(file, budget.clone()).unwrap();
        file.write_all(contents).unwrap();
        file.sync_all().unwrap();
    }
    assert_eq!(budget.used(), budget.maximum());
    let mut next = cursor(&budget);
    assert!(
        next.write_all(&[3])
            .unwrap_err()
            .to_string()
            .contains("Take-wide")
    );
    assert_eq!(next.inner.into_inner(), Vec::<u8>::new());
    assert_eq!(budget.used(), 12);
}

#[test]
fn concurrent_admission_never_oversubscribes_and_uncommitted_reservations_refund() {
    let budget = DiskBudget::new(10);
    let ready = Arc::new(Barrier::new(3));
    let admitted = Arc::new(Barrier::new(3));
    let release = Arc::new(Barrier::new(3));
    let workers: Vec<_> = (0..2)
        .map(|_| {
            let (budget, ready, admitted, release) = (
                budget.clone(),
                ready.clone(),
                admitted.clone(),
                release.clone(),
            );
            std::thread::spawn(move || {
                ready.wait();
                let reservation = budget.reserve(6);
                admitted.wait();
                release.wait();
                reservation.is_ok()
            })
        })
        .collect();
    ready.wait();
    admitted.wait();
    assert_eq!(budget.used(), 6);
    release.wait();
    assert_eq!(
        workers
            .into_iter()
            .map(|worker| usize::from(worker.join().unwrap()))
            .sum::<usize>(),
        1
    );
    assert_eq!(budget.used(), 0);
    let overflow = DiskBudget::new(u64::MAX);
    overflow.adopt(u64::MAX).unwrap();
    assert!(overflow.adopt(1).is_err());
    assert_eq!(overflow.used(), u64::MAX);
}

struct PartialWriter {
    remaining: usize,
}
impl Write for PartialWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.remaining == 0 {
            return Err(io::Error::other("injected write failure"));
        }
        let count = bytes.len().min(self.remaining);
        self.remaining -= count;
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn short_writes_failures_and_overwrites_charge_only_actual_growth() {
    let budget = DiskBudget::new(8);
    let mut writer = Metered {
        inner: PartialWriter { remaining: 3 },
        budget: budget.clone(),
        extent: Arc::new(AtomicU64::new(0)),
        position: 0,
    };
    assert_eq!(writer.write(&[1; 8]).unwrap(), 3);
    assert_eq!(budget.used(), 3);
    assert!(writer.write(&[2; 5]).is_err());
    assert_eq!(budget.used(), 3);
    let mut other = cursor(&budget);
    other.write_all(&[4; 5]).unwrap();
    other.seek(SeekFrom::Start(1)).unwrap();
    other.write_all(&[7; 2]).unwrap();
    assert_eq!(other.inner.get_ref(), &[4, 7, 7, 4, 4]);
    assert_eq!(budget.used(), 8);
    other.position = u64::MAX;
    assert!(other.write(&[0]).is_err());
    assert_eq!(budget.used(), 8);
}

#[test]
fn truncation_refunds_only_after_success_and_rewrites_need_no_free_allowance() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("audio");
    let budget = DiskBudget::new(8);
    let mut file = Metered::file(File::create(&path).unwrap(), budget.clone()).unwrap();
    file.write_all(&[1; 8]).unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(&[2; 4]).unwrap();
    assert_eq!(budget.used(), 8);
    file.set_len(3).unwrap();
    assert_eq!(budget.used(), 3);
    drop(file);
    let mut read_only = Metered::file(File::open(&path).unwrap(), DiskBudget::new(8)).unwrap();
    assert!(read_only.set_len(0).is_err());
    assert_eq!(read_only.budget.used(), 3);
    assert_eq!(std::fs::metadata(path).unwrap().len(), 3);
}

#[test]
fn staging_cleanup_and_no_clobber_publication_have_physical_refund_semantics() {
    let temporary = tempfile::tempdir().unwrap();
    let budget = DiskBudget::new(16);
    let destination = temporary.path().join("immutable");
    let mut staged = StagedFile::new(temporary.path(), "stage-", ".part", budget.clone()).unwrap();
    staged.writer().unwrap().write_all(&[1; 4]).unwrap();
    staged.writer().unwrap().write_all(&[2; 4]).unwrap();
    staged.sync_all().unwrap();
    staged.persist_noclobber(&destination).unwrap();
    assert_eq!(budget.used(), 8);
    assert_eq!(
        std::fs::read(&destination).unwrap(),
        [vec![1; 4], vec![2; 4]].concat()
    );
    let mut duplicate =
        StagedFile::new(temporary.path(), "stage-", ".part", budget.clone()).unwrap();
    duplicate.writer().unwrap().write_all(&[3; 8]).unwrap();
    assert_eq!(budget.used(), 16);
    let error = duplicate.persist_noclobber(&destination).unwrap_err();
    assert_eq!(error.error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(
        budget.used(),
        16,
        "failed publication still owns its physical staging"
    );
    drop(error);
    assert_eq!(budget.used(), 8);
    assert_eq!(std::fs::read(&destination).unwrap()[0], 1);
    assert_eq!(std::fs::read_dir(temporary.path()).unwrap().count(), 1);
}

#[test]
fn replacements_charge_both_copies_until_publication_and_retain_failed_complete_outputs() {
    let temporary = tempfile::tempdir().unwrap();
    let budget = DiskBudget::new(12);
    let destination = temporary.path().join("video");
    let mut original = Metered::file(File::create(&destination).unwrap(), budget.clone()).unwrap();
    original.write_all(&[1; 4]).unwrap();
    drop(original);
    let mut staged = StagedFile::new(temporary.path(), "mux-", ".part", budget.clone()).unwrap();
    staged.writer().unwrap().write_all(&[2; 8]).unwrap();
    assert_eq!(budget.used(), 12);
    staged.sync_all().unwrap();
    staged.persist_replace(&destination).unwrap();
    assert_eq!(budget.used(), 8);
    assert_eq!(std::fs::read(&destination).unwrap(), vec![2; 8]);
    let mut failed = StagedFile::new(temporary.path(), "mux-", ".part", budget.clone()).unwrap();
    failed.writer().unwrap().write_all(&[3; 4]).unwrap();
    failed.sync_all().unwrap();
    let error = failed
        .persist_replace(&temporary.path().join("missing"))
        .unwrap_err();
    let notice = error.file.retain(error.error.to_string());
    assert!(notice.contains("completed media remains"));
    assert_eq!(budget.used(), 12);
    assert_eq!(std::fs::read_dir(temporary.path()).unwrap().count(), 2);
}

#[test]
fn failed_asset_encoding_returns_staging_bytes_without_touching_published_objects() {
    let temporary = tempfile::tempdir().unwrap();
    let budget = DiskBudget::new(16);
    let mut assets =
        crate::object_store::AssetWriter::open_with_disk(temporary.path(), 16, budget.clone())
            .unwrap();
    let first = assets
        .put(4, |output| {
            output.write_all(&[1; 4]).unwrap();
            Ok(())
        })
        .unwrap();
    assert_eq!(budget.used(), 4);
    assert!(
        assets
            .put(12, |output| {
                output.write_all(&[2; 12]).unwrap();
                Err(crate::archive::ArchiveError::Cancelled)
            })
            .is_err()
    );
    assert_eq!(budget.used(), 4);
    let duplicate = assets
        .put(4, |output| {
            output.write_all(&[1; 4]).unwrap();
            Ok(())
        })
        .unwrap();
    assert_eq!(duplicate, first);
    assert_eq!(budget.used(), 4);
    assert_eq!(assets.stored_bytes(), 4);
}
