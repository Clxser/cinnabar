// STUDIO_PARAMETERS
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var linear_sampler: sampler;
@group(0) @binding(2) var<uniform> settings: Parameters;

struct VertexOutput { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex fn vertex(@builtin(vertex_index) index: u32) -> VertexOutput {
    let p = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return VertexOutput(vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0), vec2<f32>(p.x, 1.0 - p.y));
}

fn premultiplied_texel(point: vec2<i32>) -> vec4<f32> {
    let maximum = vec2<i32>(textureDimensions(source)) - vec2<i32>(1);
    let pixel = textureLoad(source, clamp(point, vec2<i32>(0), maximum), 0);
    return vec4<f32>(pixel.rgb * pixel.a, pixel.a);
}

fn sampled(uv: vec2<f32>) -> vec4<f32> {
    if settings.control.y > 0.5 {
        return textureSampleLevel(source, linear_sampler, uv, 0.0);
    }
    // Premultiply before filtering so transparent image pixels cannot tint their visible neighbours.
    let coordinate = uv * vec2<f32>(textureDimensions(source)) - 0.5;
    let point = vec2<i32>(floor(coordinate));
    let fraction = fract(coordinate);
    let top = mix(premultiplied_texel(point), premultiplied_texel(point + vec2<i32>(1, 0)), fraction.x);
    let bottom = mix(premultiplied_texel(point + vec2<i32>(0, 1)), premultiplied_texel(point + vec2<i32>(1, 1)), fraction.x);
    return mix(top, bottom, fraction.y);
}

@fragment fn layer(input: VertexOutput) -> @location(0) vec4<f32> {
    let moved = (input.uv - 0.5) / settings.motion.x + 0.5 + settings.motion.yz;
    let local = vec2<f32>(dot(settings.inverse_x.xyz, vec3<f32>(moved, 1.0)), dot(settings.inverse_y.xyz, vec3<f32>(moved, 1.0)));
    if any(local < vec2<f32>(0.0)) || any(local > vec2<f32>(1.0)) { discard; }
    let half_texel = settings.control.zw * 0.5;
    let middle = (settings.crop.xy + settings.crop.zw) * 0.5;
    let low = min(settings.crop.xy + half_texel, middle);
    let high = max(settings.crop.zw - half_texel, middle);
    let uv = clamp(mix(settings.crop.xy, settings.crop.zw, local), low, high);
    let pixel = sampled(uv);
    let alpha = clamp(pixel.a * settings.control.x, 0.0, 1.0);
    let rgb = pixel.rgb * settings.control.x;
    return vec4<f32>(rgb, alpha);
}
