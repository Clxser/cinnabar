use super::*;

/// The acknowledged empty slot exists before an autosave can write any project bytes.
fn begin(store: &mut Store, root: &Path) -> String {
    let RecoveryValue::Opened { id } = store
        .execute(RecoveryRequest::Begin { root: root.into() })
        .unwrap()
    else {
        panic!("slot acknowledgement");
    };
    id
}

/// Each snapshot has a distinct project value so fallback verifies actual recovered contents.
fn save(store: &mut Store, root: &Path, id: &str, revision: u64) {
    store
        .execute(RecoveryRequest::Save {
            root: root.into(),
            id: id.into(),
            document: 1,
            revision,
            project: Arc::new(Project::new(format!("revision {revision}"))),
            original_file: Some(root.join("original.cinstudio")),
            source_base: root.into(),
        })
        .unwrap();
}

#[test]
fn bounded_backups_restore_a_dirty_copy_and_keep_the_original_saved_bytes() {
    let root = tempfile::tempdir().unwrap();
    let original = root.path().join("original.cinstudio");
    fs::write(&original, b"the original saved project").unwrap();
    let mut store = Store::default();
    let id = begin(&mut store, root.path());
    for revision in 1..=MAX_RECOVERY_VERSIONS as u64 + 1 {
        save(&mut store, root.path(), &id, revision);
    }
    let listed = files::list(root.path()).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].versions, MAX_RECOVERY_VERSIONS);
    let RecoveryValue::Restored { editor, .. } = store
        .execute(RecoveryRequest::Restore {
            root: root.path().into(),
            id,
        })
        .unwrap()
    else {
        panic!("restored editor");
    };
    assert!(editor.dirty());
    assert_eq!(
        editor.project().name,
        format!("revision {}", MAX_RECOVERY_VERSIONS + 1)
    );
    assert_eq!(fs::read(original).unwrap(), b"the original saved project");
}

#[test]
fn intentional_discard_retires_the_path_before_acknowledging_and_rejects_late_writes() {
    let root = tempfile::tempdir().unwrap();
    let mut store = Store::default();
    let id = begin(&mut store, root.path());
    save(&mut store, root.path(), &id, 1);
    store
        .execute(RecoveryRequest::DiscardDocument {
            root: root.path().into(),
            document: 1,
            known: Some(id.clone()),
            restored: None,
        })
        .unwrap();
    assert!(files::list(root.path()).unwrap().is_empty());
    let snapshot = Snapshot {
        version: RECOVERY_VERSION,
        revision: 2,
        modified_unix_ms: 0,
        original_file: None,
        project: Project::new("must not resurrect"),
    };
    assert!(files::save(&files::slot(root.path(), &id).unwrap(), &snapshot).is_err());
    assert!(!files::slot(root.path(), &id).unwrap().exists());
    assert!(
        store
            .execute(RecoveryRequest::Save {
                root: root.path().into(),
                id,
                document: 1,
                revision: 2,
                project: Arc::new(Project::new("late write")),
                original_file: None,
                source_base: root.path().into()
            })
            .is_err()
    );
}

#[test]
fn saved_and_restored_checkpoints_do_not_delete_newer_snapshot_revisions() {
    let root = tempfile::tempdir().unwrap();
    let mut store = Store::default();
    let id = begin(&mut store, root.path());
    save(&mut store, root.path(), &id, 1);
    save(&mut store, root.path(), &id, 2);
    let value = store
        .execute(RecoveryRequest::Saved {
            root: root.path().into(),
            document: 1,
            revision: 1,
            known: Some(id.clone()),
            restored: None,
        })
        .unwrap();
    assert!(matches!(
        value,
        RecoveryValue::Checkpoint { retained: true }
    ));
    let (latest, _, versions) = reader::latest(&files::slot(root.path(), &id).unwrap()).unwrap();
    assert_eq!(latest.revision, 2);
    assert_eq!(versions, 1);
    let mut recovered = Store::default();
    recovered
        .execute(RecoveryRequest::DiscardDocument {
            root: root.path().into(),
            document: 1,
            known: None,
            restored: Some((id.clone(), 1)),
        })
        .unwrap();
    assert_eq!(
        reader::latest(&files::slot(root.path(), &id).unwrap())
            .unwrap()
            .0
            .revision,
        2
    );
}

#[test]
fn corrupt_latest_falls_back_explicitly_and_observed_discard_keeps_future_writes() {
    let root = tempfile::tempdir().unwrap();
    let mut store = Store::default();
    let id = begin(&mut store, root.path());
    save(&mut store, root.path(), &id, 1);
    save(&mut store, root.path(), &id, 2);
    let slot = files::slot(root.path(), &id).unwrap();
    fs::write(&files::versions(&slot).unwrap()[0].1, b"broken metadata").unwrap();
    let listed = files::list(root.path()).unwrap();
    assert_eq!(listed[0].revision, 1);
    assert_eq!(listed[0].discard_through, 2);
    assert!(listed[0].notice.is_some());
    save(&mut store, root.path(), &id, 3);
    store
        .execute(RecoveryRequest::Discard {
            root: root.path().into(),
            id: id.clone(),
            revision: listed[0].discard_through,
        })
        .unwrap();
    assert_eq!(reader::latest(&slot).unwrap().0.revision, 3);
    let mut stale_writer = Store::default();
    assert!(matches!(
        stale_writer.execute(RecoveryRequest::Save {
            root: root.path().into(),
            id: id.clone(),
            document: 1,
            revision: 2,
            project: Arc::new(Project::new("discarded writer")),
            original_file: None,
            source_base: root.path().into(),
        }),
        Err(StorageError::RecoveryRetired)
    ));
    assert_eq!(reader::latest(&slot).unwrap().0.revision, 3);
    assert!(files::slot(root.path(), "../saved-project").is_err());
}

#[test]
fn another_session_holding_the_lease_returns_busy_without_creating_a_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let guard = lease::acquire(root.path()).unwrap();
    let mut store = Store::default();
    assert!(matches!(
        store.execute(RecoveryRequest::Begin {
            root: root.path().into()
        }),
        Err(StorageError::Busy)
    ));
    drop(guard);
    let id = begin(&mut store, root.path());
    save(&mut store, root.path(), &id, 1);
    assert_eq!(files::list(root.path()).unwrap().len(), 1);
}

#[test]
fn failed_admission_does_not_replace_an_existing_saved_project() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("saved.cinstudio");
    let original = Project::new("last saved");
    super::super::files::save(&path, &original, root.path()).unwrap();
    let before = fs::read(&path).unwrap();
    let mut invalid = original;
    invalid.version = crate::model::PROJECT_VERSION + 1;
    assert!(super::super::files::save(&path, &invalid, root.path()).is_err());
    assert_eq!(fs::read(path).unwrap(), before);
}

#[test]
fn durable_discard_threshold_survives_interruption_before_snapshot_deletion() {
    let root = tempfile::tempdir().unwrap();
    let mut store = Store::default();
    let id = begin(&mut store, root.path());
    save(&mut store, root.path(), &id, 1);
    save(&mut store, root.path(), &id, 2);
    let slot = files::slot(root.path(), &id).unwrap();
    {
        let _lease = lease::acquire(root.path()).unwrap();
        lease::discard_through(&slot, 2).unwrap();
    }
    assert_eq!(files::versions(&slot).unwrap().len(), 2);
    assert!(reader::latest(&slot).is_err());
    assert!(!files::list(root.path()).unwrap()[0].recoverable);
    assert!(
        store
            .execute(RecoveryRequest::Restore {
                root: root.path().into(),
                id: id.clone()
            })
            .is_err()
    );
    save(&mut store, root.path(), &id, 3);
    let (latest, _, count) = reader::latest(&slot).unwrap();
    assert_eq!(latest.revision, 3);
    assert_eq!(count, 1);
}

#[test]
fn a_retained_copy_can_resume_only_with_a_fresh_document_generation() {
    let root = tempfile::tempdir().unwrap();
    let mut store = Store::default();
    let id = begin(&mut store, root.path());
    save(&mut store, root.path(), &id, 1);
    store
        .execute(RecoveryRequest::DiscardDocument {
            root: root.path().into(),
            document: 1,
            known: Some(id),
            restored: None,
        })
        .unwrap();
    let id = begin(&mut store, root.path());
    let request = |document| RecoveryRequest::Save {
        root: root.path().into(),
        id: id.clone(),
        document,
        revision: 1,
        project: Arc::new(Project::new("retained copy")),
        original_file: None,
        source_base: root.path().into(),
    };
    assert!(matches!(
        store.execute(request(1)),
        Err(StorageError::RecoveryRetired)
    ));
    store.execute(request(2)).unwrap();
    assert_eq!(
        reader::latest(&files::slot(root.path(), &id).unwrap())
            .unwrap()
            .0
            .project
            .name,
        "retained copy"
    );
}
