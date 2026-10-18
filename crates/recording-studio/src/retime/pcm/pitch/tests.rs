use super::*;
use crate::{
    PlaybackRate,
    audio::sample_frame,
    retime::{SpeedKey, SpeedRamp},
};
use std::io::Cursor;

#[test]
fn every_monitoring_speed_keeps_the_tone_audible_and_at_its_original_pitch() {
    let rate = 48_000;
    let padding = PitchWarp::<Cursor<Vec<u8>>>::padding_frames(rate).unwrap();
    let bytes: Vec<_> = (0..rate as usize + padding)
        .flat_map(|frame| {
            let value = if frame < rate as usize {
                (std::f64::consts::TAU * 500.0 * frame as f64 / f64::from(rate)).sin() as f32
            } else {
                0.0
            };
            [value, value].into_iter().flat_map(f32::to_le_bytes)
        })
        .collect();
    for &speed in crate::playback::monitoring::MonitoringSpeed::ALL {
        let ratio = speed.rate();
        let frames = u64::from(rate) * u64::from(ratio.denominator) / u64::from(ratio.numerator);
        let mut warp = PitchWarp::monitoring(Cursor::new(bytes.as_slice()), ratio, rate).unwrap();
        let mut previous = 0.0;
        let mut crossings = 0;
        let mut energy = 0.0;
        let mut count = 0;
        for frame in 0..frames {
            let value = warp.sample(frame).unwrap()[0];
            if frame >= 1024 && frame < frames - 1024 {
                crossings += usize::from(previous <= 0.0 && value > 0.0);
                energy += f64::from(value) * f64::from(value);
                count += 1;
            }
            previous = value;
        }
        let pitch = crossings as f64 * f64::from(rate) / count as f64;
        assert!(energy / count as f64 > 0.1, "{} was silent", speed.label());
        assert!(
            (pitch - 500.0).abs() < 20.0,
            "{} changed pitch to {pitch}",
            speed.label()
        );
        assert_eq!(warp.next, frames);
        assert!(warp.source.len() <= warp.grain * 3);
    }
}

#[test]
fn a_speed_ramp_preserves_tone_pitch_without_accumulating_output_clock_error() {
    let rate = 48_000;
    let source = Span::new(MediaTime::ZERO, MediaTime(1_000_000)).unwrap();
    let ramp = SpeedRamp {
        keys: vec![
            SpeedKey {
                at: source.start,
                rate: PlaybackRate {
                    numerator: 1,
                    denominator: 2,
                },
                easing: Easing::Linear,
            },
            SpeedKey {
                at: source.end,
                rate: PlaybackRate {
                    numerator: 2,
                    denominator: 1,
                },
                easing: Easing::Linear,
            },
        ],
    };
    let map = TimeMap::new(
        source,
        MediaTime::ZERO,
        PlaybackRate::default(),
        Some(&ramp),
    )
    .unwrap();
    let frames = sample_frame(map.span().end, rate).unwrap();
    let padding = PitchWarp::<Cursor<Vec<u8>>>::padding_frames(rate).unwrap();
    let mut bytes = Vec::new();
    for frame in 0..rate as usize + padding {
        let sample = if frame < rate as usize {
            (std::f64::consts::TAU * 500.0 * frame as f64 / f64::from(rate)).sin() as f32
        } else {
            0.0
        };
        for _ in 0..CHANNELS {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
    }
    let mut warp = PitchWarp::new(Cursor::new(bytes), map, MediaTime::ZERO, rate, 0).unwrap();
    let output: Vec<_> = (0..frames)
        .map(|frame| warp.sample(frame).unwrap()[0])
        .collect();
    let retained = &output[4096..output.len() - 4096];
    let crossings = retained
        .windows(2)
        .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
        .count();
    let pitch = crossings as f64 * f64::from(rate) / retained.len() as f64;
    assert!((pitch - 500.0).abs() < 10.0, "pitch was {pitch}");
    assert_eq!(warp.next, frames);
    assert!(warp.source.len() <= warp.grain * 3);
    assert!(warp.sample(frames - 1).is_err());
}
