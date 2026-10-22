use super::*;

/// Queue admission can be exercised deterministically before any source worker consumes its request.
fn reader(
    request: &TargetCoverageRequest,
    cancellation: &Cancellation,
) -> Result<Option<TargetCoverage>, String> {
    cancellation.check()?;
    Ok(Some(TargetCoverage {
        source_range: request.source_range,
        target: request.target,
    }))
}

/// Metadata-only requests establish bounded replacement without disk or scheduling assumptions.
fn request(runtime_id: u64) -> TargetCoverageRequest {
    TargetCoverageRequest {
        directory: Default::default(),
        source_range: Span::new(MediaTime(0), MediaTime(10)).unwrap(),
        target: CameraTargetRef {
            runtime_id,
            lifetime: None,
        },
        require_eye: false,
    }
}

#[test]
fn replacing_queued_work_cancels_and_disconnects_its_result_without_growing_the_queue() {
    let (sender, waiting) = crossbeam_channel::bounded(1);
    let verifier = TargetVerifier { sender, waiting };
    let first_cancel = Cancellation::default();
    let first = verifier
        .submit(reader, request(1), first_cancel.clone())
        .unwrap();
    let second_cancel = Cancellation::default();
    let second = verifier
        .submit(reader, request(2), second_cancel.clone())
        .unwrap();
    assert!(first_cancel.cancelled());
    assert!(matches!(
        first.try_recv(),
        Err(crossbeam_channel::TryRecvError::Disconnected)
    ));
    assert!(!second_cancel.cancelled());
    let work = verifier.waiting.try_recv().unwrap();
    assert!(verifier.waiting.is_empty());
    work.result
        .try_send((work.reader)(&work.request, &work.cancellation))
        .unwrap();
    assert_eq!(
        second
            .try_recv()
            .unwrap()
            .unwrap()
            .unwrap()
            .target
            .runtime_id,
        2
    );
}

#[test]
fn dropping_the_verifier_cancels_queued_sources_without_a_join() {
    let (sender, waiting) = crossbeam_channel::bounded(1);
    let verifier = TargetVerifier { sender, waiting };
    let cancellation = Cancellation::default();
    let result = verifier
        .submit(reader, request(1), cancellation.clone())
        .unwrap();
    drop(verifier);
    assert!(cancellation.cancelled());
    assert!(matches!(
        result.try_recv(),
        Err(crossbeam_channel::TryRecvError::Disconnected)
    ));
}
