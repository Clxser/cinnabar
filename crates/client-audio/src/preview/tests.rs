use super::*;

fn block(generation: u64, start: u64, len: usize, value: f32) -> PreviewBlock {
    PreviewBlock {
        generation,
        start,
        len,
        samples: [[value; 2]; PREVIEW_BLOCK_FRAMES],
    }
}

fn route(output: &PreviewOutput) -> Routed<rodio::buffer::SamplesBuffer<f32>> {
    Routed::new(
        rodio::buffer::SamplesBuffer::new(PREVIEW_CHANNELS, output.sample_rate(), vec![0.9; 512]),
        output.clone(),
    )
}

#[test]
fn exclusive_preview_mutes_live_audio_and_pause_does_not_consume_pcm() {
    let output = PreviewOutput::new(48_000);
    let generation = output.reset(0, 4);
    output.set_exclusive(true);
    output.push(block(generation, 0, 4, 0.25)).ok().unwrap();
    let mut source = route(&output);
    assert_eq!(source.next(), Some(0.0));
    assert_eq!(source.next(), Some(0.0));
    assert_eq!(output.position(), 0);
    output.set_playing(true, 4);
    assert_eq!(source.next(), Some(0.25));
    assert_eq!(source.next(), Some(0.25));
    assert_eq!(output.position(), 1);
    output.set_playing(false, 4);
    assert_eq!(source.next(), Some(0.0));
    assert_eq!(source.next(), Some(0.0));
    assert_eq!(output.position(), 1);
    output.set_exclusive(false);
    assert_eq!(source.next(), Some(0.9));
}

#[test]
fn pause_cursor_is_acknowledged_after_the_inflight_stereo_frame_and_invalidated_on_seek() {
    let output = PreviewOutput::new(48_000);
    let generation = output.reset(0, 4);
    output.set_exclusive(true);
    output.push(block(generation, 0, 4, 0.25)).ok().unwrap();
    output.set_playing(true, 4);
    let mut source = route(&output);
    assert_eq!(source.next(), Some(0.25));
    output.set_playing(false, 4);
    assert_eq!(output.paused_position(), None);
    assert_eq!(source.next(), Some(0.0));
    assert_eq!(output.paused_position(), None);
    assert_eq!(source.next(), Some(0.0));
    assert_eq!(output.paused_position(), Some(1));
    output.set_playing(true, 4);
    assert_eq!(output.paused_position(), None);
    source.by_ref().take(2).for_each(drop);
    output.set_playing(false, 4);
    assert_eq!(output.paused_position(), None);
    source.by_ref().take(2).for_each(drop);
    assert_eq!(output.paused_position(), Some(2));
    output.reset(100, 104);
    assert_eq!(output.paused_position(), None);
    source.by_ref().take(2).for_each(drop);
    assert_eq!(output.paused_position(), Some(100));
}

#[test]
fn video_backpressure_stalls_both_clock_and_audio_without_dropping_the_next_sample() {
    let output = PreviewOutput::new(48_000);
    let generation = output.reset(100, 104);
    output.set_exclusive(true);
    output.set_playing(true, 101);
    output.push(block(generation, 100, 4, 0.5)).ok().unwrap();
    let mut source = route(&output);
    assert_eq!(source.by_ref().take(2).collect::<Vec<_>>(), [0.5, 0.5]);
    assert_eq!(source.by_ref().take(6).collect::<Vec<_>>(), [0.0; 6]);
    assert_eq!(output.position(), 101);
    output.set_playing(true, 104);
    assert_eq!(source.by_ref().take(2).collect::<Vec<_>>(), [0.5, 0.5]);
    assert_eq!(output.position(), 102);
}

#[test]
fn seek_retires_queued_and_half_consumed_old_generation_samples() {
    let output = PreviewOutput::new(48_000);
    let old = output.reset(0, 4);
    output.set_exclusive(true);
    output.set_playing(true, 4);
    output.push(block(old, 0, 4, 0.25)).ok().unwrap();
    let mut source = route(&output);
    assert_eq!(source.next(), Some(0.25));
    let next = output.reset(200, 204);
    assert_eq!(output.position(), 200);
    assert_eq!(source.next(), Some(0.0));
    assert!(output.push(block(old, 0, 4, 0.75)).is_err());
    output.push(block(next, 200, 4, 0.5)).ok().unwrap();
    output.set_playing(true, 204);
    assert_eq!(source.next(), Some(0.5));
    assert_eq!(source.next(), Some(0.5));
    assert_eq!(output.position(), 201);
}

#[test]
fn queue_and_finite_sample_admission_are_bounded() {
    let output = PreviewOutput::new(48_000);
    let generation = output.reset(0, 100);
    for index in 0..QUEUED_BLOCKS {
        assert!(output.push(block(generation, index as u64, 1, 0.5)).is_ok());
    }
    assert!(output.push(block(generation, 8, 1, 0.5)).is_err());
    let generation = output.reset(0, 100);
    assert!(!output.full());
    assert!(output.push(block(generation, 0, 1, f32::NAN)).is_err());
    assert!(output.push(block(generation, 0, 0, 0.0)).is_err());
}

#[test]
fn a_video_ceiling_stall_does_not_hide_a_later_pcm_underrun() {
    let output = PreviewOutput::new(48_000);
    let generation = output.reset(0, 4);
    output.set_exclusive(true);
    output.set_playing(true, 1);
    output.push(block(generation, 0, 1, 0.5)).ok().unwrap();
    let mut source = route(&output);
    source.by_ref().take(4).for_each(drop);
    assert_eq!(output.underruns(), 0);
    output.set_playing(true, 4);
    source.by_ref().take(4).for_each(drop);
    assert_eq!(output.position(), 1);
    assert_eq!(output.underruns(), 1);
}

#[test]
fn monitoring_gain_preserves_the_pcm_clock_and_clamps_after_gain() {
    let output = PreviewOutput::new(48_000);
    let generation = output.reset(0, 2);
    output.set_exclusive(true);
    output.set_playing(true, 2);
    output.set_gain(0.25);
    output.push(block(generation, 0, 2, 2.0)).ok().unwrap();
    let mut source = route(&output);
    assert_eq!(source.next(), Some(0.5));
    assert_eq!(source.next(), Some(0.5));
    output.set_gain(0.0);
    assert_eq!(source.next(), Some(0.0));
    assert_eq!(source.next(), Some(0.0));
    assert_eq!(output.position(), 2);
}
