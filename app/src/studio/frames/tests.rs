use super::*;

/// Tiny owned pixels exercise admission and ordering without a renderer or encoder process.
fn sample(index: u64) -> Frame {
    Frame {
        index,
        size: [2, 2],
        layout: PixelLayout::Rgba,
        data: vec![index as u8; 16],
    }
}

#[test]
fn completed_readbacks_retry_a_full_encoder_without_losing_order_or_permits() {
    let mut readbacks = Readbacks::default();
    let first = readbacks.reserve([2, 2]).unwrap().unwrap();
    let second = readbacks.reserve([2, 2]).unwrap().unwrap();
    second.complete(Ok(sample(1)));
    assert_eq!(
        readbacks
            .drain(|_| panic!("earlier frame is still pending"))
            .unwrap(),
        0
    );
    first.complete(Ok(sample(0)));
    assert_eq!(
        readbacks
            .drain(|frame| Err(FramePushError::Full(frame)))
            .unwrap(),
        0
    );
    assert_eq!(readbacks.pending(), 2);
    assert_eq!(
        readbacks.bytes.load(Ordering::Acquire),
        2 * sample(0).data.len()
    );
    assert_eq!(readbacks.deliver, 0);
    let mut written = Vec::new();
    assert_eq!(
        readbacks
            .drain(|frame| {
                written.push(frame.index);
                Ok(())
            })
            .unwrap(),
        2
    );
    assert_eq!(written, [0, 1]);
    assert_eq!(readbacks.pending(), 0);
    assert_eq!(readbacks.bytes.load(Ordering::Acquire), 0);
}

#[test]
fn encoder_backpressure_cannot_grow_the_readback_window() {
    let mut readbacks = Readbacks::default();
    for index in 0..MAX_READBACKS {
        readbacks
            .reserve([2, 2])
            .unwrap()
            .unwrap()
            .complete(Ok(sample(index as u64)));
    }
    for _ in 0..100 {
        assert_eq!(
            readbacks
                .drain(|frame| Err(FramePushError::Full(frame)))
                .unwrap(),
            0
        );
        assert!(readbacks.reserve([2, 2]).unwrap().is_none());
        assert_eq!(readbacks.pending(), MAX_READBACKS);
    }
    assert_eq!(readbacks.drain(|_| Ok(())).unwrap(), MAX_READBACKS as u64);
    assert_eq!(readbacks.pending(), 0);
}

#[test]
fn abandoned_observer_releases_its_slot_without_blocking_later_frames() {
    let mut readbacks = Readbacks::default();
    let abandoned = readbacks.reserve([2, 2]).unwrap().unwrap();
    readbacks
        .reserve([2, 2])
        .unwrap()
        .unwrap()
        .complete(Ok(sample(1)));
    drop(abandoned);
    let mut written = Vec::new();
    assert_eq!(
        readbacks
            .drain(|frame| {
                written.push(frame.index);
                Ok(())
            })
            .unwrap(),
        1
    );
    assert_eq!(written, [1]);
    assert_eq!(readbacks.pending(), 0);
    assert_eq!(readbacks.readback_stalls, 1);
    assert_eq!(readbacks.bytes.load(Ordering::Acquire), 0);
}

#[test]
fn failed_readback_does_not_pin_the_remaining_cleanup_sequence() {
    let mut readbacks = Readbacks::default();
    readbacks
        .reserve([2, 2])
        .unwrap()
        .unwrap()
        .complete(Err("mapping failed".into()));
    readbacks
        .reserve([2, 2])
        .unwrap()
        .unwrap()
        .complete(Ok(sample(1)));
    assert_eq!(
        readbacks
            .drain(|_| panic!("failed readback cannot reach encoder"))
            .unwrap_err(),
        "mapping failed"
    );
    assert_eq!(readbacks.pending(), 1);
    let mut written = Vec::new();
    assert_eq!(
        readbacks
            .drain(|frame| {
                written.push(frame.index);
                Ok(())
            })
            .unwrap(),
        1
    );
    assert_eq!(written, [1]);
    assert_eq!(readbacks.pending(), 0);
}
