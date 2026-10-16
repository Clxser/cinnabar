// STUDIO_PARAMETERS
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var glow: texture_2d<f32>;
@group(0) @binding(2) var linear_sampler: sampler;
@group(0) @binding(3) var<uniform> settings: Parameters;

struct VertexOutput { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex fn vertex(@builtin(vertex_index) index: u32) -> VertexOutput {
    let p = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return VertexOutput(vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0), vec2<f32>(p.x, 1.0 - p.y));
}

fn scene(uv: vec2<f32>) -> vec3<f32> {
    let moved = (uv - 0.5) / settings.motion.x + 0.5
        + vec2<f32>(settings.motion.y / settings.motion.w, settings.motion.z);
    if any(moved < vec2<f32>(0.0)) || any(moved > vec2<f32>(1.0)) { return vec3<f32>(0.0); }
    return textureSampleLevel(source, linear_sampler, moved, 0.0).rgb * settings.tone.x;
}

fn bright(uv: vec2<f32>) -> vec3<f32> {
    let value = scene(uv);
    let luminance = dot(value, vec3<f32>(0.2126, 0.7152, 0.0722));
    let knee = smoothstep(0.55, 1.0, luminance);
    return value * knee;
}

@fragment fn extract(input: VertexOutput) -> @location(0) vec4<f32> {
    let step = settings.step.xy * 0.5;
    let value = bright(input.uv + vec2<f32>(-step.x, -step.y))
        + bright(input.uv + vec2<f32>(step.x, -step.y))
        + bright(input.uv + vec2<f32>(-step.x, step.y))
        + bright(input.uv + step);
    return vec4<f32>(value * 0.25, 1.0);
}

fn blur(uv: vec2<f32>, step: vec2<f32>) -> vec3<f32> {
    var result = textureSampleLevel(glow, linear_sampler, uv, 0.0).rgb * 6.0;
    result += (textureSampleLevel(glow, linear_sampler, uv - step, 0.0).rgb
        + textureSampleLevel(glow, linear_sampler, uv + step, 0.0).rgb) * 4.0;
    result += textureSampleLevel(glow, linear_sampler, uv - step * 2.0, 0.0).rgb
        + textureSampleLevel(glow, linear_sampler, uv + step * 2.0, 0.0).rgb;
    return result / 16.0;
}

@fragment fn horizontal(input: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(blur(input.uv, vec2<f32>(settings.step.x, 0.0)), 1.0);
}

@fragment fn grade(input: VertexOutput) -> @location(0) vec4<f32> {
    let uv = input.uv;
    var color = scene(uv);
    if settings.tone.w > 0.0 { color += blur(uv, vec2<f32>(0.0, settings.step.y)) * settings.tone.w; }
    let luminance = dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
    color = mix(vec3<f32>(luminance), color, settings.tone.y);
    let radius = dot(uv - 0.5, uv - 0.5) * 2.0;
    color *= 1.0 - settings.tone.z * smoothstep(0.15, 1.0, radius);
    color = mix(color, vec3<f32>(1.0), settings.overlay.x);
    if uv.y < settings.overlay.y || uv.y > 1.0 - settings.overlay.y { color = vec3<f32>(0.0); }
    color *= 1.0 - settings.overlay.z;
    return vec4<f32>(max(color, vec3<f32>(0.0)), 1.0);
}
