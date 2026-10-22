use super::*;
use crate::{
    archive::{RecordInput, RecordingState},
    automation::GameplayEventKind,
    capture::companions::fixture::{bookmark, checkpoint, event, recording},
    model::{Confidence, MarkerKind},
};
use std::sync::Arc;

#[test]
fn the_primary_cursor_recovers_bookmarks_before_and_after_later_checkpoints() {
    let root = tempfile::tempdir().unwrap();
    let mut recording = recording(root.path());
    event(
        &mut recording,
        MediaTime(2_000_000),
        1,
        MediaTime(2_000_000),
        bookmark("Before checkpoint"),
    );
    checkpoint(&mut recording, MediaTime(5_000_000));
    event(
        &mut recording,
        MediaTime(6_000_000),
        2,
        MediaTime(6_000_000),
        bookmark("After checkpoint"),
    );
    let task = recording.finish(MediaTime(8_000_000)).unwrap();
    assert_eq!(task.wait().state, RecordingState::Complete);
    let events = read(
        task.path(),
        MediaTime(7_000_000),
        2,
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(events.observed, 2);
    assert_eq!(
        events
            .highlights
            .iter()
            .map(|event| event.label.as_str())
            .collect::<Vec<_>>(),
        ["Before checkpoint", "After checkpoint"]
    );
    assert_eq!(events.omitted, 0);
}

#[test]
fn sequence_gaps_duplicates_zero_and_payload_timestamp_mismatches_are_rejected() {
    for (sequence, at) in [(0, 2), (1, 2), (3, 2), (2, 1)] {
        let root = tempfile::tempdir().unwrap();
        let mut recording = recording(root.path());
        event(
            &mut recording,
            MediaTime(1),
            1,
            MediaTime(1),
            bookmark("First"),
        );
        event(
            &mut recording,
            MediaTime(2),
            sequence,
            MediaTime(at),
            bookmark("Malformed"),
        );
        let task = recording.finish(MediaTime(3)).unwrap();
        task.wait();
        // A full marker budget must not stop validation of later durable events.
        assert!(read(task.path(), MediaTime(3), 1, &Cancellation::default()).is_err());
    }
}

#[test]
fn legacy_gap_sensitive_combat_correlations_and_combos_are_not_invented() {
    let root = tempfile::tempdir().unwrap();
    let mut recording = recording(root.path());
    let kinds = [
        GameplayEventKind::AttackIntent { target: 9 },
        GameplayEventKind::Hurt {
            victim: 9,
            attacker: None,
            player: true,
        },
        GameplayEventKind::Death {
            victim: 9,
            attacker: None,
            player: true,
        },
        GameplayEventKind::Hurt {
            victim: 9,
            attacker: Some(7),
            player: true,
        },
        GameplayEventKind::Hurt {
            victim: 9,
            attacker: Some(7),
            player: true,
        },
        GameplayEventKind::Hurt {
            victim: 9,
            attacker: Some(7),
            player: true,
        },
        GameplayEventKind::Death {
            victim: 9,
            attacker: Some(7),
            player: true,
        },
    ];
    for (index, kind) in kinds.into_iter().enumerate() {
        let at = MediaTime(1_000 + index as u64 * 1_000);
        event(&mut recording, at, index as u64 + 1, at, kind);
    }
    let task = recording.finish(MediaTime(10_000)).unwrap();
    task.wait();
    let events = read(task.path(), MediaTime(10_000), 20, &Cancellation::default()).unwrap();
    let kinds = events
        .highlights
        .iter()
        .map(|event| (event.kind, event.confidence))
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        [
            (MarkerKind::Death, Confidence::Observed),
            (MarkerKind::Hit, Confidence::Confirmed),
            (MarkerKind::Hit, Confidence::Confirmed),
            (MarkerKind::Hit, Confidence::Confirmed),
            (MarkerKind::Kill, Confidence::Confirmed),
        ]
    );
    assert!(events.highlights.iter().all(|event| event.combo <= 1));
    assert_eq!(events.observed, 7);
}

#[test]
fn rejected_event_tail_never_enters_the_recovered_committed_interval() {
    let root = tempfile::tempdir().unwrap();
    let mut recording = recording(root.path());
    event(
        &mut recording,
        MediaTime(1_000),
        1,
        MediaTime(1_000),
        bookmark("Committed"),
    );
    recording
        .push(RecordInput {
            at: MediaTime(2_000),
            kind: RecordKind::Camera,
            payload: Arc::from([]),
        })
        .unwrap();
    let task = recording.task();
    assert!(
        recording
            .push(RecordInput {
                at: MediaTime(crate::time::MAX_MEDIA_MICROS + 1),
                kind: RecordKind::Gameplay,
                payload: Arc::from(
                    serde_json::to_vec(&GameplayEvent {
                        sequence: 2,
                        at: MediaTime(3_000),
                        kind: bookmark("Rejected")
                    })
                    .unwrap()
                ),
            })
            .is_err()
    );
    assert!(recording.finish(MediaTime(4_000)).is_err());
    assert_eq!(task.wait().state, RecordingState::Failed);
    let events = read(task.path(), MediaTime(2_000), 2, &Cancellation::default()).unwrap();
    assert_eq!(events.highlights.len(), 1);
    assert_eq!(events.highlights[0].label, "Committed");
    assert_eq!(events.observed, 1);
}

#[test]
fn half_open_prefix_and_retention_budget_count_omissions_without_retiming_events() {
    let root = tempfile::tempdir().unwrap();
    let mut recording = recording(root.path());
    for index in 1..=3 {
        let at = MediaTime(index * 1_000);
        event(&mut recording, at, index, at, bookmark("Durable"));
    }
    let task = recording.finish(MediaTime(4_000)).unwrap();
    task.wait();
    let events = read(task.path(), MediaTime(3_000), 1, &Cancellation::default()).unwrap();
    assert_eq!(events.observed, 3);
    assert_eq!(events.omitted, 1);
    assert_eq!(events.highlights.len(), 1);
    assert_eq!(events.highlights[0].at, MediaTime(1_000));
    let cancellation = Cancellation::default();
    assert!(cancellation.cancel());
    assert!(read(task.path(), MediaTime(3_000), 3, &cancellation).is_err());
}
