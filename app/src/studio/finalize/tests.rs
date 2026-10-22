use super::*;

#[test]
fn native_failure_reports_only_the_independently_verified_common_prefix() {
    let failure = native_failure_report("Terrain queue failed", || Ok(MediaTime(1_250_000)));
    assert_eq!(failure.verified_prefix, Some(MediaTime(1_250_000)));
    assert!(failure.message.contains("1.250s"));
    assert!(failure.message.contains("Terrain queue failed"));
    assert!(failure.message.contains("Recover this interrupted take"));
}

#[test]
fn unavailable_or_invalid_native_assets_never_claim_a_verified_or_successful_save() {
    let failure = native_failure_report("Native capture interrupted", || {
        Err("A material object is unavailable".into())
    });
    assert_eq!(failure.verified_prefix, None);
    assert!(failure.message.contains("could not be verified"));
    assert!(failure.message.contains("material object"));
    assert!(failure.message.contains("original journals are retained"));
}

#[test]
fn a_zero_length_prefix_is_reported_as_unplayable() {
    let failure = native_failure_report("Baseline interrupted", || Ok(MediaTime::ZERO));
    assert_eq!(failure.verified_prefix, None);
    assert!(
        failure
            .message
            .contains("No common committed playback interval")
    );
}

#[test]
fn finalizer_failure_publishes_the_verified_duration_without_claiming_success() {
    let mut state = NativeCapture::default();
    state.status.phase = recording_studio::capture::CapturePhase::Finishing;
    state.status.elapsed = MediaTime(9_000_000);
    let (completed, finishing) = bounded(1);
    state.finishing = Some(finishing);
    completed
        .send(Err(native_failure_report("Terrain failed", || {
            Ok(MediaTime(1_250_000))
        })))
        .unwrap();
    super::super::poll_finish(&mut state);
    assert_eq!(
        state.status.phase,
        recording_studio::capture::CapturePhase::Failed
    );
    assert_eq!(state.status.elapsed, MediaTime(1_250_000));
    assert!(
        state
            .status
            .message
            .as_deref()
            .unwrap()
            .contains("Verified common committed prefix")
    );
    assert!(state.finishing.is_none());
}
