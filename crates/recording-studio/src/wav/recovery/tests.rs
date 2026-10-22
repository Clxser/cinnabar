use super::*;
use std::{cell::Cell, fs::OpenOptions};

/// These bytes come from the production writer; clearing ds64 models an interrupted header publication.
fn fixture(directory: &Path, frames: usize, rate: u32, unfinished: bool) -> PathBuf {
    let path = directory.join(crate::capture::AUDIO_FILE);
    let mut writer = WavWriter::create_rf64(&path, 2, rate).unwrap();
    writer
        .write((0..frames * 2).map(|sample| (sample % 32) as f32 / 32.0))
        .unwrap();
    writer.finish().unwrap();
    if unfinished {
        let mut file = OpenOptions::new().write(true).open(&path).unwrap();
        file.seek(SeekFrom::Start(20)).unwrap();
        file.write_all(&[0; 24]).unwrap();
        file.sync_all().unwrap();
    }
    path
}

#[test]
fn unfinished_header_and_partial_frame_recover_only_complete_physical_samples_without_touching_source()
 {
    let root = tempfile::tempdir().unwrap();
    let path = fixture(root.path(), 4, 1_000, true);
    OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(&[1, 2, 3])
        .unwrap();
    let original = std::fs::read(&path).unwrap();
    let prepared = prepare(&path, MediaTime(10_000), &Cancellation::default()).unwrap();
    assert_eq!(prepared.frames, 4);
    assert_eq!(prepared.duration, MediaTime(4_000));
    assert_eq!(prepared.discarded_bytes, 3);
    assert!(prepared.unfinished);
    let published = prepared
        .publish(root.path(), &Cancellation::default())
        .unwrap();
    let recovered = std::fs::read(&published.path).unwrap();
    assert_eq!(recovered.len(), RF64_HEADER_BYTES as usize + 4 * 8);
    let header = RF64_HEADER_BYTES as usize;
    assert_eq!(&recovered[header..], &original[header..header + 4 * 8]);
    assert_eq!(
        u64::from_le_bytes(recovered[28..36].try_into().unwrap()),
        4 * 8
    );
    assert_eq!(u64::from_le_bytes(recovered[36..44].try_into().unwrap()), 4);
    assert_eq!(std::fs::read(&path).unwrap(), original);
    published.retain();
}

#[test]
fn a_truncated_finalized_payload_retains_only_complete_available_frames_and_reports_the_missing_tail()
 {
    let root = tempfile::tempdir().unwrap();
    let path = fixture(root.path(), 4, 1_000, false);
    OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(RF64_HEADER_BYTES + 2 * 8 + 3)
        .unwrap();
    let original = std::fs::read(&path).unwrap();
    let prepared = prepare(&path, MediaTime(10_000), &Cancellation::default()).unwrap();
    assert_eq!(prepared.frames, 2);
    assert_eq!(prepared.duration, MediaTime(2_000));
    assert!(prepared.truncated);
    assert!(!prepared.unfinished);
    assert_eq!(prepared.discarded_bytes, 3);
    assert_eq!(std::fs::read(&path).unwrap(), original);
}

#[test]
fn prefix_duration_uses_complete_source_sample_frames_under_rational_capture_clocks() {
    for rate in [44_100_u32, 48_000] {
        for fps in [
            crate::FrameRate {
                numerator: 30_000,
                denominator: 1001,
            },
            crate::FrameRate {
                numerator: 60,
                denominator: 1,
            },
        ] {
            let root = tempfile::tempdir().unwrap();
            let path = fixture(root.path(), rate as usize, rate, false);
            let prefix = fps.frame_time(3).unwrap();
            let prepared = prepare(&path, prefix, &Cancellation::default()).unwrap();
            let frames =
                (u128::from(prefix.0) * u128::from(rate) / u128::from(MICROS_PER_SECOND)) as u64;
            assert_eq!(prepared.frames, frames);
            assert_eq!(
                prepared.duration,
                MediaTime(
                    (u128::from(frames) * u128::from(MICROS_PER_SECOND) / u128::from(rate)) as u64
                )
            );
            assert!(prepared.duration <= prefix);
            assert_eq!(
                std::fs::metadata(prepared.path()).unwrap().len(),
                RF64_HEADER_BYTES + frames * 8
            );
        }
    }
}

#[test]
fn malformed_or_partially_finalized_headers_are_explained_and_leave_no_staging_files() {
    for (offset, bytes) in [
        (0, vec![0]),
        (58, vec![0, 0]),
        (70, vec![16, 0]),
        (64, vec![0; 4]),
        (20, vec![1; 8]),
    ] {
        let root = tempfile::tempdir().unwrap();
        let path = fixture(root.path(), 2, 1_000, true);
        let mut file = OpenOptions::new().write(true).open(&path).unwrap();
        file.seek(SeekFrom::Start(offset)).unwrap();
        file.write_all(&bytes).unwrap();
        drop(file);
        let original = std::fs::read(&path).unwrap();
        assert!(prepare(&path, MediaTime(10_000), &Cancellation::default()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }
}

#[test]
fn cancelled_mid_copy_drops_partial_output_and_preserves_original_pcm() {
    let root = tempfile::tempdir().unwrap();
    let frames = crate::audio::MAX_BLOCK_FRAMES * 2;
    let path = fixture(root.path(), frames, 48_000, true);
    let original = std::fs::read(&path).unwrap();
    let checks = Cell::new(0);
    let result = prepare_checked(&path, MediaTime(1_000_000), &|| {
        checks.set(checks.get() + 1);
        if checks.get() == 3 {
            Err("Fixture cancellation".into())
        } else {
            Ok(())
        }
    });
    assert_eq!(result.err().as_deref(), Some("Fixture cancellation"));
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn recovery_does_not_clobber_a_previous_attempt_and_abandoned_publication_is_removed() {
    let root = tempfile::tempdir().unwrap();
    let path = fixture(root.path(), 4, 1_000, true);
    let first = root.path().join("Recovered game audio 1.wav");
    std::fs::write(&first, b"previous recovery").unwrap();
    let prepared = prepare(&path, MediaTime(3_000), &Cancellation::default()).unwrap();
    let published = prepared
        .publish(root.path(), &Cancellation::default())
        .unwrap();
    assert_eq!(
        published.path,
        root.path().join("Recovered game audio 2.wav")
    );
    assert_eq!(std::fs::read(&first).unwrap(), b"previous recovery");
    let recovered = published.path.clone();
    drop(published);
    assert!(!recovered.exists());
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 2);
}

#[test]
fn cancellation_before_publication_and_io_failure_clean_up_the_staging_file() {
    let root = tempfile::tempdir().unwrap();
    let path = fixture(root.path(), 4, 1_000, false);
    let cancellation = Cancellation::default();
    let prepared = prepare(&path, MediaTime(3_000), &cancellation).unwrap();
    cancellation.cancel();
    assert!(prepared.publish(root.path(), &cancellation).is_err());
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    let prepared = prepare(&path, MediaTime(3_000), &Cancellation::default()).unwrap();
    assert!(prepared.publish(&path, &Cancellation::default()).is_err());
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn an_empty_header_or_non_finite_pcm_cannot_be_admitted_as_recovered_audio() {
    let root = tempfile::tempdir().unwrap();
    let path = fixture(root.path(), 0, 48_000, true);
    assert!(prepare(&path, MediaTime(1_000), &Cancellation::default()).is_err());
    let mut writer = WavWriter::create_rf64(&path, 2, 48_000).unwrap();
    writer.write([f32::NAN, 0.0]).unwrap();
    writer.finish().unwrap();
    assert!(prepare(&path, MediaTime(1_000), &Cancellation::default()).is_err());
}

#[test]
fn an_appending_capture_is_rejected_instead_of_importing_a_mutable_audio_sidecar() {
    let root = tempfile::tempdir().unwrap();
    let path = fixture(
        root.path(),
        crate::audio::MAX_BLOCK_FRAMES * 2,
        48_000,
        true,
    );
    let checks = Cell::new(0);
    let result = prepare_checked(&path, MediaTime(1_000_000), &|| {
        checks.set(checks.get() + 1);
        if checks.get() == 3 {
            OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap()
                .write_all(&[0; 8])
                .unwrap();
        }
        Ok(())
    });
    assert!(result.err().unwrap().contains("changed during recovery"));
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
}
