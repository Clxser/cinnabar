use super::*;
use recording_studio::model::FramingFit;

#[test]
fn gpu_layer_uniforms_use_the_same_rotated_crop_transform_as_cpu_sampling() {
    let framing = Framing {
        crop: [0.1, 0.2, 0.8, 0.9],
        opacity: 0.75,
        rotation_degrees: 31.25,
        fit: FramingFit::Fill,
    };
    let source = [400, 200];
    let output = [320, 240];
    let transform = FrameTransform::new(framing, source, output).unwrap();
    let uniforms = parameters_for(framing, source, output, false, [1.0, 0.0, 0.0]).unwrap();
    assert_eq!(uniforms.crop, transform.crop);
    assert_eq!(uniforms.inverse_x, transform.inverse_x);
    assert_eq!(uniforms.inverse_y, transform.inverse_y);
    assert_eq!(uniforms.control[0], 0.75);
    let moved = parameters_for(framing, source, output, false, [1.5, 0.2, -0.1]).unwrap();
    assert_eq!(moved.inverse_x, uniforms.inverse_x);
    assert_eq!(moved.inverse_y, uniforms.inverse_y);
    assert_eq!(moved.motion[0], 1.5);
    assert!((moved.motion[1] - 0.15).abs() < 0.00001);
    assert_eq!(moved.motion[2], -0.1);
}
