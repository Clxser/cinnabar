struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) opacity: f32,
};
@group(0) @binding(0) var ink: texture_2d<f32>;
@group(0) @binding(1) var ink_sampler: sampler;

@vertex fn vertex(@location(0) position: vec2<f32>, @location(1) uv: vec2<f32>, @location(2) opacity: f32) -> VertexOutput {
    return VertexOutput(vec4<f32>(position, 0.0, 1.0), uv, opacity);
}
@fragment fn fragment(input: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(ink, ink_sampler, input.uv) * input.opacity;
}
