use super::*;

/// Metadata-only state exercises worker completions without opening storage or starting a thread.
fn studio() -> Studio {
    let mut studio = Studio::new("unused".into());
    studio.install(
        Editor::recovered(Project::new("current edit")).unwrap(),
        None,
    );
    studio
}

/// Synthetic entries carry real stable identities while their presentation order remains mutable.
fn entry(id: &str) -> recording_studio::storage::RecoveryEntry {
    recording_studio::storage::RecoveryEntry {
        id: id.into(),
        name: id.into(),
        original_file: None,
        revision: 2,
        discard_through: 2,
        modified_unix_ms: 0,
        versions: 1,
        notice: None,
        recoverable: true,
    }
}

/// A completion is admitted through the same job token used by the asynchronous storage owner.
fn complete(studio: &mut Studio, job: u64, operation: Operation, value: RecoveryValue) {
    studio.recovery.pending = Some((job, operation));
    studio.recovery_completed(StorageResult {
        job,
        result: Ok(StorageValue::Recovery(value)),
    });
}

#[test]
fn successful_restore_exits_loading_and_installs_a_dirty_copy() {
    let mut studio = studio();
    studio.begin_restore("project-chosen".into()).unwrap();
    let operation = studio.recovery.desired.take().unwrap();
    complete(
        &mut studio,
        1,
        operation,
        RecoveryValue::Restored {
            id: "project-chosen".into(),
            revision: 2,
            editor: Editor::recovered(Project::new("chosen copy")).unwrap(),
            notice: "recovered".into(),
        },
    );
    assert!(!studio.view.loading);
    assert!(!studio.view.workflow.recovery.busy);
    assert!(!studio.view.workflow.recovery.visible);
    assert!(studio.editor.as_ref().unwrap().dirty());
    assert_eq!(studio.project().unwrap().name, "chosen copy");
    assert!(studio.view.file.is_none());
}

#[test]
fn canceled_restore_retains_edits_warns_across_scans_and_explicit_retry_gets_a_new_generation() {
    let mut studio = studio();
    let original = Arc::clone(studio.project().unwrap());
    let document = studio.document;
    studio.recovery.retired = true;
    studio.begin_restore("project-other".into()).unwrap();
    studio.recovery.pending = Some((7, studio.recovery.desired.take().unwrap()));
    studio.activate(Action::CancelOpen);
    assert!(Arc::ptr_eq(studio.project().unwrap(), &original));
    assert!(!studio.view.loading);
    assert!(studio.recovery.retired);
    assert!(studio.view.workflow.recovery.error.is_some());
    studio.recovery_completed(StorageResult {
        job: 7,
        result: Ok(StorageValue::Recovery(RecoveryValue::Restored {
            id: "project-other".into(),
            revision: 2,
            editor: Editor::recovered(Project::new("must not install")).unwrap(),
            notice: String::new(),
        })),
    });
    assert!(Arc::ptr_eq(studio.project().unwrap(), &original));
    complete(
        &mut studio,
        8,
        Operation::Scan,
        RecoveryValue::Entries(Vec::new()),
    );
    assert!(studio.view.workflow.recovery.error.is_some());
    studio.save_job = Some(persistence::SaveJob {
        job: 10,
        document,
        project: Arc::clone(&original),
    });
    studio.retry_autosave();
    assert_ne!(studio.document, document);
    assert!(!studio.recovery.retired);
    assert_eq!(studio.save_job.as_ref().unwrap().document, studio.document);
    assert!(Arc::ptr_eq(studio.project().unwrap(), &original));
    assert!(studio.recovery.clock.advance(studio.document, 0, true, 0.0));
    let document = studio.document;
    complete(
        &mut studio,
        9,
        Operation::Save(document, 0),
        RecoveryValue::Saved {
            document,
            revision: 0,
        },
    );
    assert!(studio.view.workflow.recovery.error.is_none());
}

#[test]
fn confirmed_restore_keeps_its_captured_identity_after_list_reordering() {
    let mut studio = studio();
    Arc::make_mut(&mut studio.view).workflow.recovery.entries =
        vec![entry("project-first"), entry("project-second")].into();
    studio.activate(Action::RestoreRecovery(0));
    assert_eq!(
        studio.view.discard,
        Some(DiscardIntent::RestoreRecovery("project-first".into()))
    );
    Arc::make_mut(&mut studio.view).workflow.recovery.entries =
        vec![entry("project-second"), entry("project-first")].into();
    studio.activate(Action::ConfirmDiscard);
    let operation = studio.recovery.desired.take().unwrap();
    complete(&mut studio, 1, operation, RecoveryValue::Cleared);
    assert!(
        matches!(&studio.recovery.desired, Some(Operation::Restore { id, .. }) if id == "project-first")
    );
}

#[test]
fn autosave_completion_does_not_reveal_a_list_while_its_scan_is_queued() {
    let mut studio = studio();
    let document = studio.document;
    Arc::make_mut(&mut studio.view).workflow.recovery.entries = vec![entry("project-first")].into();
    studio.show_recovery();
    complete(
        &mut studio,
        1,
        Operation::Save(document, 0),
        RecoveryValue::Saved {
            document,
            revision: 0,
        },
    );
    assert!(studio.view.workflow.recovery.loading);
    assert!(!studio.focus_actions().iter().any(|action| matches!(
        action,
        launcher::menu::MenuAction::Studio(Action::RestoreRecovery(_) | Action::DiscardRecovery(_))
    )));
    studio.recovery.scan = false;
    complete(
        &mut studio,
        2,
        Operation::Scan,
        RecoveryValue::Entries(Vec::new()),
    );
    assert!(!studio.view.workflow.recovery.loading);
}

#[test]
fn failed_replacement_keeps_the_retired_editor_and_offers_explicit_recovery_retry() {
    let mut studio = studio();
    studio.recovery.retired = true;
    studio.open_job = Some(3);
    Arc::make_mut(&mut studio.view).loading = true;
    studio.completed(StorageResult {
        job: 3,
        result: Err(recording_studio::storage::StorageError::Closed),
    });
    assert!(!studio.view.loading);
    assert_eq!(studio.project().unwrap().name, "current edit");
    assert!(studio.editor.as_ref().unwrap().dirty());
    assert!(studio.view.workflow.recovery.error.is_some());
}

#[test]
fn an_unresolved_checkpoint_warning_survives_other_write_and_scan_completions() {
    let mut studio = studio();
    let document = studio.document;
    let checkpoint = Operation::Checkpoint {
        document,
        revision: 0,
        known: None,
        restored: None,
    };
    studio.recovery.failed_checkpoint = Some(checkpoint.clone());
    studio.record_recovery_warning(&checkpoint, "cleanup needs retry");
    studio.record_recovery_warning(&Operation::Save(document, 1), "write also failed");
    complete(
        &mut studio,
        1,
        Operation::Save(document, 1),
        RecoveryValue::Saved {
            document,
            revision: 1,
        },
    );
    complete(
        &mut studio,
        2,
        Operation::Scan,
        RecoveryValue::Entries(Vec::new()),
    );
    assert!(studio.view.workflow.recovery.error.is_some());
    assert!(studio.recovery.failed_checkpoint.is_some());
    studio.retry_autosave();
    let checkpoint = studio.recovery.checkpoint.take().unwrap();
    complete(
        &mut studio,
        3,
        checkpoint,
        RecoveryValue::Checkpoint { retained: false },
    );
    assert!(studio.view.workflow.recovery.error.is_none());
}
