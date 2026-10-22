use super::*;
use crate::{disk::DiskBudget, object_store::AssetWriter, wav::WavWriter};

/// Logical physical extents include unindexed partials and retained immutable objects.
fn physical_bytes(path: &Path) -> u64 {
    std::fs::read_dir(path)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let metadata = std::fs::symlink_metadata(&path).unwrap();
            if metadata.is_dir() {
                physical_bytes(&path)
            } else {
                metadata.len()
            }
        })
        .sum()
}

/// Every quota fixture uses a well-formed source header independent of platform or game assets.
fn info() -> RecordingInfo {
    RecordingInfo {
        version: RECORDING_VERSION,
        name: "Aggregate allowance fixture".into(),
        game_version: "fixture".into(),
        protocol_version: 1,
        scene_schema: "fixture/1".into(),
        created_unix_ms: 0,
        viewport: [16, 16],
        local_actor: 1,
    }
}

/// A completed zero-time checkpoint is recoverable even when a later stream consumes all free bytes.
fn baseline(journal: &mut writer::Writer) {
    for (index, kind) in [RecordKind::CheckpointStart, RecordKind::CheckpointEnd]
        .into_iter()
        .enumerate()
    {
        journal
            .append(
                index as u64 + 1,
                &RecordInput {
                    at: MediaTime::ZERO,
                    kind,
                    payload: Arc::from([]),
                },
            )
            .unwrap();
    }
}

#[test]
fn assets_pcm_and_journals_cannot_each_spend_the_full_take_allowance() {
    let temporary = tempfile::tempdir().unwrap();
    let budget = DiskBudget::new(32 * 1024);
    let root = temporary.path().join("primary");
    let config = RecordingConfig {
        compression_level: 0,
        ..Default::default()
    };
    let mut journal =
        writer::Writer::create_with_disk(root.clone(), &info(), config, budget.clone()).unwrap();
    baseline(&mut journal);
    let mut audio = WavWriter::create_rf64_with_disk(
        &temporary.path().join("game.wav"),
        2,
        48_000,
        budget.clone(),
    )
    .unwrap();
    audio.write([0.5; 16]).unwrap();
    audio.finish().unwrap();
    let mut assets = AssetWriter::open_with_disk(&root, budget.maximum(), budget.clone()).unwrap();
    let fill = (budget.maximum() - budget.used() - 64) as usize;
    assets
        .put(fill as u64, |out| {
            out.write_all(&vec![7; fill])
                .map_err(|error| io(&root, error))
        })
        .unwrap();
    let appended = journal.append(
        3,
        &RecordInput {
            at: MediaTime(1),
            kind: RecordKind::Scene,
            payload: vec![9; 1024].into(),
        },
    );
    let error = match appended {
        Ok(()) => journal.complete(MediaTime(1)).unwrap_err(),
        Err(error) => {
            drop(journal);
            error
        }
    };
    assert!(error.to_string().contains("Take-wide"));
    let reader = RecordingReader::open(&root).unwrap();
    assert!(reader.recovery().interrupted);
    assert_eq!(reader.duration(), MediaTime::ZERO);
    let mut kinds = Vec::new();
    reader
        .visit_to(
            MediaTime::ZERO,
            || false,
            |record| {
                kinds.push(record.kind);
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(
        kinds,
        [RecordKind::CheckpointStart, RecordKind::CheckpointEnd]
    );
    assert_eq!(physical_bytes(temporary.path()), budget.used());
    assert!(budget.used() <= budget.maximum());
}

#[test]
fn rejected_completion_marker_leaves_a_readable_common_prefix() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("primary");
    let budget = DiskBudget::new(16 * 1024);
    let mut journal = writer::Writer::create_with_disk(
        root.clone(),
        &info(),
        RecordingConfig::default(),
        budget.clone(),
    )
    .unwrap();
    baseline(&mut journal);
    let mut assets = AssetWriter::open_with_disk(&root, budget.maximum(), budget.clone()).unwrap();
    let fill = (budget.maximum() - budget.used()) as usize;
    assets
        .put(fill as u64, |out| {
            out.write_all(&vec![1; fill])
                .map_err(|error| io(&root, error))
        })
        .unwrap();
    assert!(
        journal
            .complete(MediaTime::ZERO)
            .unwrap_err()
            .to_string()
            .contains("Take-wide")
    );
    assert!(!root.join(COMPLETE_FILE).exists());
    let reader = RecordingReader::open(&root).unwrap();
    assert!(reader.recovery().interrupted);
    assert_eq!(physical_bytes(temporary.path()), budget.used());
    assert_eq!(budget.used(), budget.maximum());
}
