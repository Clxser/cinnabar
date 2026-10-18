use super::*;

#[test]
fn stored_and_compressed_segments_preserve_identical_records_and_seek_boundaries() {
    let temporary = tempfile::tempdir().unwrap();
    let info = RecordingInfo {
        version: RECORDING_VERSION,
        name: "Compression fixture".into(),
        game_version: "fixture".into(),
        protocol_version: 1,
        scene_schema: "fixture/1".into(),
        created_unix_ms: 0,
        viewport: [16, 16],
        local_actor: 1,
    };
    let payload: Arc<[u8]> = (0..200_000)
        .map(|index| (index % 251) as u8)
        .collect::<Vec<_>>()
        .into();
    for compression_level in [0, 1, 9] {
        let root = temporary.path().join(compression_level.to_string());
        let mut writer = writer::Writer::create(
            root.clone(),
            &info,
            RecordingConfig {
                compression_level,
                ..Default::default()
            },
        )
        .unwrap();
        let records = [
            (MediaTime::ZERO, RecordKind::CheckpointStart, Arc::from([])),
            (
                MediaTime::ZERO,
                RecordKind::CheckpointPart,
                Arc::clone(&payload),
            ),
            (MediaTime::ZERO, RecordKind::CheckpointEnd, Arc::from([])),
            (MediaTime(300_000), RecordKind::Scene, Arc::clone(&payload)),
        ];
        for (index, (at, kind, payload)) in records.iter().enumerate() {
            writer
                .append(
                    index as u64 + 1,
                    &RecordInput {
                        at: *at,
                        kind: *kind,
                        payload: Arc::clone(payload),
                    },
                )
                .unwrap();
        }
        writer.complete(MediaTime(600_000)).unwrap();
        let reader = RecordingReader::open(&root).unwrap();
        let mut actual = Vec::new();
        reader
            .visit_to(
                MediaTime(600_000),
                || false,
                |record| {
                    actual.push((record.at, record.kind, record.payload.to_vec()));
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(actual.len(), records.len());
        for (actual, expected) in actual.iter().zip(&records) {
            assert_eq!(
                (actual.0, actual.1, actual.2.as_slice()),
                (expected.0, expected.1, &*expected.2)
            );
        }
        let mut scenes = 0;
        reader
            .visit_to(
                MediaTime(200_000),
                || false,
                |record| {
                    scenes += usize::from(record.kind == RecordKind::Scene);
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(scenes, 0);
    }
}

#[test]
fn compression_policy_rejects_invalid_levels_before_storage() {
    assert!(
        RecordingConfig {
            compression_level: 10,
            ..Default::default()
        }
        .validate()
        .is_err()
    );
    assert!(
        RecordingConfig {
            compression_level: 0,
            ..Default::default()
        }
        .validate()
        .is_ok()
    );
}
