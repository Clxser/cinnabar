use super::*;
use crate::{
    Id, PlaybackRate, Span,
    model::{AudioClip, AudioRole, MediaKind, MediaSource},
    playback::Transport,
};
use std::sync::Arc;

fn clip() -> AudioClip {
    AudioClip {
        id: Id(2),
        source: Id(1),
        linked_clip: None,
        source_range: Span::new(MediaTime::ZERO, MediaTime(1_000_000)).unwrap(),
        at: MediaTime::ZERO,
        rate: PlaybackRate::default(),
        speed_ramp: None,
        role: AudioRole::Game,
        gain: 1.0,
        fade_in: MediaTime::ZERO,
        fade_out: MediaTime::ZERO,
        muted: false,
    }
}

#[test]
fn sample_clock_uses_half_open_boundaries_and_rejects_invalid_rates() {
    assert_eq!(sample_frame(MediaTime(1), 48_000).unwrap(), 1);
    assert_eq!(sample_frame(MediaTime(1_000_000), 48_000).unwrap(), 48_000);
    assert_eq!(sample_time(1, 48_000).unwrap(), MediaTime(20));
    assert!(sample_frame(MediaTime(1), 0).is_err());
    assert!(sample_time(1, MAX_SAMPLE_RATE + 1).is_err());
}

#[test]
fn mixing_applies_gain_fades_and_mute_on_the_absolute_output_clock() {
    let mut clip = clip();
    clip.gain = 0.5;
    clip.fade_in = MediaTime(1_000_000);
    let span = clip.span().unwrap();
    let mut output = [[0.0; 2]; 1];
    mix_into(&clip, span, 24_000, 48_000, &[[1.0; 2]], &mut output).unwrap();
    assert_eq!(output, [[0.25; 2]]);
    mix_into(&clip, span, 24_000, 48_000, &[[1.0; 2]], &mut output).unwrap();
    assert_eq!(output, [[0.5; 2]]);
    clip.muted = true;
    mix_into(&clip, span, 24_000, 48_000, &[[1.0; 2]], &mut output).unwrap();
    assert_eq!(output, [[0.5; 2]]);
}

#[test]
fn audio_edits_preserve_linked_source_clock_and_bound_fades() {
    let mut original = clip();
    original.linked_clip = Some(Id(9));
    original.gain = MAX_GAIN;
    let changed = AudioParameter::FadeIn.adjusted(&original, 1).unwrap();
    assert_eq!(changed.fade_in, MediaTime(100_000));
    assert_eq!(changed.source_range, original.source_range);
    assert_eq!(changed.linked_clip, original.linked_clip);
    assert_eq!(changed.rate, original.rate);
    assert_eq!(changed.speed_ramp, original.speed_ramp);
    assert_eq!(
        AudioParameter::Gain.adjusted(&original, 1).unwrap().gain,
        MAX_GAIN
    );
    let mut changed = original.clone();
    changed.fade_out = changed.span().unwrap().duration();
    assert_eq!(
        AudioParameter::FadeOut
            .adjusted(&changed, 1)
            .unwrap()
            .fade_out,
        changed.fade_out
    );
}

#[test]
fn muted_audio_outputs_silence_without_opening_missing_source_files() {
    let mut project = crate::Project::new("silent");
    project.sources.push(MediaSource {
        id: Id(1),
        name: "missing".into(),
        path: "intentionally-missing-audio.wav".into(),
        kind: MediaKind::Audio,
        duration: MediaTime(1_000_000),
        size: [0, 0],
        frame_rate: None,
        sha256: None,
    });
    let mut audio = clip();
    audio.muted = true;
    project.audio.push(audio);
    let mut mixer = Mixer::new(Arc::new(project), std::path::Path::new("."), 48_000, 0).unwrap();
    let mut output = [[1.0; 2]; 8];
    mixer
        .read(&mut output, 0, &crate::export::Cancellation::default())
        .unwrap();
    assert_eq!(output, [[0.0; 2]; 8]);
}

#[test]
fn audio_clock_never_rewinds_or_advances_beyond_displayed_video() {
    let mut transport = Transport::default();
    let duration = MediaTime(1_000_000);
    transport.play(duration);
    let epoch = transport.epoch();
    transport.synchronize_audio(MediaTime(500_000), duration, MediaTime(100_000));
    assert_eq!(transport.position(), MediaTime(100_000));
    transport.synchronize_audio(MediaTime(50_000), duration, MediaTime(50_000));
    assert_eq!(transport.position(), MediaTime(100_000));
    assert_eq!(transport.epoch(), epoch);
    transport.synchronize_audio(duration, duration, duration);
    assert_eq!(transport.position(), duration);
    assert!(!transport.playing());
}
