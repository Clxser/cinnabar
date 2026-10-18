use super::*;

#[test]
fn exclusive_studio_pcm_never_enters_the_game_recording_tap() {
    let shared = tap::SharedTap::new(MIXER_RATE);
    let recording = shared.begin().unwrap();
    let output = crate::preview::PreviewOutput::new(MIXER_RATE);
    let frames = TAP_BLOCK_SAMPLES / CAPTURE_CHANNELS as usize;
    let generation = output.reset(0, frames as u64);
    let mut block = crate::preview::PreviewBlock {
        generation,
        start: 0,
        len: frames,
        samples: [[0.0; 2]; crate::preview::PREVIEW_BLOCK_FRAMES],
    };
    block.samples[..frames].fill([0.25; 2]);
    output.push(block).ok().unwrap();
    output.set_exclusive(true);
    output.set_playing(true, frames as u64);
    let game = rodio::buffer::SamplesBuffer::new(
        CAPTURE_CHANNELS,
        MIXER_RATE,
        vec![0.75; TAP_BLOCK_SAMPLES],
    );
    let mut source = crate::preview::Routed::new(tap::Tapped::new(game, shared), output);
    assert!(
        source
            .by_ref()
            .take(TAP_BLOCK_SAMPLES)
            .all(|sample| sample == 0.25)
    );
    let recorded = recording.pop().expect("full game audio block");
    assert_eq!(recorded.len, TAP_BLOCK_SAMPLES);
    assert!(
        recorded.samples[..recorded.len]
            .iter()
            .all(|sample| *sample == 0.75)
    );
}
