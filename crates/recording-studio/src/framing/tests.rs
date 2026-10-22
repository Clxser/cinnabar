use super::*;
use crate::timeline::FramingParameter;

#[test]
fn legacy_framing_keeps_fit_and_zero_rotation_and_new_values_round_trip() {
    let legacy: Framing =
        serde_json::from_str(r#"{"crop":[0.0,0.0,1.0,1.0],"opacity":1.0}"#).unwrap();
    assert_eq!(legacy, Framing::default());
    let framing = Framing {
        rotation_degrees: -37.25,
        fit: FramingFit::Fill,
        ..legacy
    };
    framing.validate().unwrap();
    let encoded = serde_json::to_string(&framing).unwrap();
    assert_eq!(serde_json::from_str::<Framing>(&encoded).unwrap(), framing);
    for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(
            Framing {
                rotation_degrees: invalid,
                ..legacy
            }
            .validate()
            .is_err()
        );
    }
    assert!(
        FrameTransform::new(
            Framing {
                rotation_degrees: f64::MAX,
                ..legacy
            },
            [40, 20],
            [60, 40]
        )
        .is_ok()
    );
}

#[test]
fn fill_covers_every_canvas_pixel_after_arbitrary_crop_rotation_without_stretching() {
    let source = [400, 200];
    let output = [64, 48];
    for rotation_degrees in [0.0, 31.25, 45.0, 90.0, -37.1, 125.0] {
        let framing = Framing {
            crop: [0.1, 0.2, 0.9, 0.8],
            rotation_degrees,
            fit: FramingFit::Fill,
            ..Framing::default()
        };
        let transform = FrameTransform::new(framing, source, output).unwrap();
        for y in 0..output[1] {
            for x in 0..output[0] {
                assert!(
                    transform
                        .source_uv([
                            (x as f32 + 0.5) / output[0] as f32,
                            (y as f32 + 0.5) / output[1] as f32
                        ])
                        .is_some(),
                    "uncovered pixel {x},{y} at {rotation_degrees} degrees"
                );
            }
        }
        let [width, height] = [source[0] as f32 * 0.8, source[1] as f32 * 0.6];
        let dx = [
            transform.inverse_x[0] * width / output[0] as f32,
            transform.inverse_y[0] * height / output[0] as f32,
        ];
        let dy = [
            transform.inverse_x[1] * width / output[1] as f32,
            transform.inverse_y[1] * height / output[1] as f32,
        ];
        let length = |value: [f32; 2]| (value[0] * value[0] + value[1] * value[1]).sqrt();
        assert!((length(dx) - length(dy)).abs() < 0.0001);
        assert!((dx[0] * dy[0] + dx[1] * dy[1]).abs() < 0.001);
    }
}

#[test]
fn fit_contains_the_rotated_crop_and_rotation_controls_keep_their_units() {
    let framing = Framing {
        rotation_degrees: 45.0,
        ..Framing::default()
    };
    let transform = FrameTransform::new(framing, [400, 200], [64, 64]).unwrap();
    assert!(transform.source_uv([0.5, 0.5]).is_some());
    assert!(transform.source_uv([0.01, 0.01]).is_none());
    assert_eq!(FramingParameter::Rotation.formatted(framing), "45.0°");
    assert_eq!(FramingParameter::Opacity.formatted(framing), "100.0%");
    let changed = FramingParameter::Rotation.adjusted(framing, -1);
    assert_eq!(changed.rotation_degrees, 44.0);
    assert_eq!(changed.crop, framing.crop);
    assert_eq!(changed.opacity, framing.opacity);
    let changed = FramingParameter::Fit.adjusted(changed, 1);
    assert_eq!(changed.fit, FramingFit::Fill);
    assert_eq!(changed.rotation_degrees, 44.0);
    let wrapped = FramingParameter::Rotation.adjusted(
        Framing {
            rotation_degrees: 179.0,
            ..framing
        },
        1,
    );
    assert_eq!(wrapped.rotation_degrees, -180.0);
}
