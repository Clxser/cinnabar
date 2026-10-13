//! Measures the production terrain pipeline offscreen without touching the live world.

use super::scene;

use super::super::pipeline::layouts::{ChunkPipeline, ChunkPipelineKey};
use bevy::{
    prelude::*,
    render::{
        render_resource::{CachedRenderPipelineId, PipelineCache},
        renderer::{RenderDevice, RenderQueue},
    },
};
use render_scene::graphics_benchmark::{GraphicsBenchmark, Request, Response};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
};

const BATCH: u32 = 8;
const WARMUP: usize = 4;
const SAMPLES: usize = 100;

#[derive(Resource, Default)]
pub(super) struct State(Option<Run>);

struct Run {
    request: Request,
    scene: scene::Scene,
    pipelines: Vec<(u32, CachedRenderPipelineId)>,
    next: usize,
    targets: Option<Targets>,
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    readback: wgpu::Buffer,
    mapped: Arc<AtomicU8>,
    timings: Vec<u64>,
    results: BTreeMap<u32, u64>,
}

struct Targets {
    color: wgpu::TextureView,
    depth: wgpu::TextureView,
}

/// Checks attachment memory before allocating any offscreen textures.
fn fits(request: &Request, samples: u32, max_side: u32) -> bool {
    let [width, height] = request.resolution;
    width > 0
        && height > 0
        && width <= max_side
        && height <= max_side
        && u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|pixels| pixels.checked_mul(u64::from(samples) * 8))
            .is_some_and(|bytes| bytes <= request.memory_budget_bytes as u64 / 2)
}

/// Allocates only the current MSAA pair; previous targets drop before the next tier.
fn targets(device: &wgpu::Device, request: &Request, samples: u32) -> Targets {
    let make = |format| {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("graphics benchmark offscreen target"),
                size: wgpu::Extent3d {
                    width: request.resolution[0],
                    height: request.resolution[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default())
    };
    Targets {
        color: make(render_scene::SCENE_COLOR_FORMAT),
        depth: make(bevy::core_pipeline::core_3d::CORE_3D_DEPTH_FORMAT),
    }
}

impl Run {
    /// Requests the same specialized pipelines used for ordinary opaque terrain.
    fn new(
        request: Request,
        device: &RenderDevice,
        queue: &RenderQueue,
        cache: &PipelineCache,
        pipeline: &mut ChunkPipeline,
    ) -> Result<Self, &'static str> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return Err("GPU timestamps are not supported");
        }
        let mut pipelines = Vec::new();
        for &samples in request.sample_budgets_ns.keys() {
            if !fits(&request, samples, device.limits().max_texture_dimension_2d) {
                continue;
            }
            let msaa = match samples {
                1 => Msaa::Off,
                2 => Msaa::Sample2,
                4 => Msaa::Sample4,
                8 => Msaa::Sample8,
                _ => continue,
            };
            let id = pipeline
                .variants
                .specialize(
                    cache,
                    ChunkPipelineKey {
                        msaa,
                        hdr: false,
                        enhanced: false,
                    },
                )
                .map_err(|_| "terrain pipeline could not be specialized")?;
            pipelines.push((samples, id));
        }
        if pipelines.is_empty() {
            return Err("offscreen targets exceed the device memory budget");
        }
        let scene = scene::Scene::new(device, queue, cache, request.resolution);
        let raw = device.wgpu_device();
        let queries = raw.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("graphics benchmark timestamps"),
            ty: wgpu::QueryType::Timestamp,
            count: BATCH * 2,
        });
        let make = |usage| {
            raw.create_buffer(&wgpu::BufferDescriptor {
                label: Some("graphics benchmark timestamp readback"),
                size: u64::from(BATCH) * 16,
                usage,
                mapped_at_creation: false,
            })
        };
        Ok(Self {
            request,
            scene,
            pipelines,
            next: 0,
            targets: None,
            queries,
            resolve: make(wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC),
            readback: make(wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST),
            mapped: Arc::new(AtomicU8::new(0)),
            timings: Vec::new(),
            results: BTreeMap::new(),
        })
    }

    /// Submits at most eight fixed frames, then lets the next device poll finish the readback.
    fn submit(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        cache: &PipelineCache,
    ) -> Result<bool, &'static str> {
        let (samples, id) = self.pipelines[self.next];
        let Some(pipeline) = cache.get_render_pipeline(id) else {
            if matches!(
                cache.get_render_pipeline_state(id),
                bevy::render::render_resource::CachedPipelineState::Err(_)
            ) {
                return Err("terrain pipeline compilation failed");
            }
            return Ok(false);
        };
        let targets = self
            .targets
            .get_or_insert_with(|| targets(device.wgpu_device(), &self.request, samples));
        let mut encoder =
            device
                .wgpu_device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("graphics benchmark terrain frames"),
                });
        for sample in 0..BATCH {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("graphics benchmark terrain frame"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &targets.color,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: Some(wgpu::RenderPassTimestampWrites {
                    query_set: &self.queries,
                    beginning_of_pass_write_index: Some(sample * 2),
                    end_of_pass_write_index: Some(sample * 2 + 1),
                }),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &*self.scene.groups[0], &[0]);
            pass.set_bind_group(1, &*self.scene.groups[1], &[]);
            pass.set_vertex_buffer(0, *self.scene.offsets.slice(..));
            pass.set_index_buffer(*self.scene.indices.slice(..), wgpu::IndexFormat::Uint32);
            for (section, range) in self.scene.ranges.iter().enumerate() {
                pass.draw_indexed(
                    0..super::super::STATIC_QUAD_INDICES.len() as u32,
                    (section * 4) as i32,
                    range.clone(),
                );
            }
        }
        encoder.resolve_query_set(&self.queries, 0..BATCH * 2, &self.resolve, 0);
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.readback, 0, u64::from(BATCH) * 16);
        queue.submit([encoder.finish()]);
        self.mapped.store(1, Ordering::Release);
        let mapped = Arc::clone(&self.mapped);
        self.readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                mapped.store(if result.is_ok() { 2 } else { 3 }, Ordering::Release);
            });
        Ok(true)
    }

    /// Collects GPU timestamps after completion, discarding warmup and using nearest-rank p99.
    fn collect(&mut self, period: f32) -> Result<bool, &'static str> {
        let data = self
            .readback
            .slice(..)
            .get_mapped_range()
            .map_err(|_| "GPU timestamp range is unavailable")?;
        for pair in data.chunks_exact(16) {
            let begin = u64::from_le_bytes(pair[..8].try_into().expect("timestamp word"));
            let end = u64::from_le_bytes(pair[8..].try_into().expect("timestamp word"));
            let ns = end
                .checked_sub(begin)
                .map(|ticks| ticks as f64 * f64::from(period))
                .filter(|ns| ns.is_finite() && *ns > 0.0)
                .ok_or("invalid GPU timestamps")?;
            self.timings.push(ns.ceil() as u64);
        }
        drop(data);
        self.readback.unmap();
        self.mapped.store(0, Ordering::Release);
        if self.timings.len() < SAMPLES + WARMUP {
            return Ok(false);
        }
        let mut measured = self.timings[WARMUP..WARMUP + SAMPLES].to_vec();
        measured.sort_unstable();
        self.results.insert(
            self.pipelines[self.next].0,
            measured[(SAMPLES * 99).div_ceil(100) - 1],
        );
        let measured_p99 = *self
            .results
            .get(&self.pipelines[self.next].0)
            .expect("completed sample count");
        self.timings.clear();
        self.targets = None;
        self.next += 1;
        // A more expensive MSAA setting is conservatively omitted when the cheaper
        // pass already uses its budget. Missing measurements cannot admit a tier.
        while self.next < self.pipelines.len()
            && measured_p99 > self.request.sample_budgets_ns[&self.pipelines[self.next].0]
        {
            self.next += 1;
        }
        Ok(self.next == self.pipelines.len())
    }
}

/// Advances one bounded batch on the selected device and drops cancelled runs immediately.
pub(super) fn drive(
    mut state: ResMut<State>,
    mailbox: Res<GraphicsBenchmark>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    cache: Res<PipelineCache>,
    mut pipeline: ResMut<ChunkPipeline>,
) {
    if let Some(request) = mailbox.take_request() {
        if let Some(previous) = state.0.take() {
            previous.request.cancellation.cancel();
        }
        if request.cancellation.is_cancelled() {
            return;
        }
        let id = request.id;
        match Run::new(request, &device, &queue, &cache, &mut pipeline) {
            Ok(run) => state.0 = Some(run),
            Err(reason) => mailbox.respond(Response {
                id,
                result: Err(reason),
            }),
        }
    }
    let Some(run) = state.0.as_mut() else {
        return;
    };
    if run.request.cancellation.is_cancelled() {
        state.0 = None;
        return;
    }
    let result = match run.mapped.load(Ordering::Acquire) {
        0 => run.submit(&device, &queue, &cache).map(|_| false),
        2 => run.collect(queue.get_timestamp_period()),
        3 => Err("GPU timestamp readback failed"),
        _ => return,
    };
    match result {
        Ok(false) => {}
        Ok(true) => {
            let run = state.0.take().expect("completed run");
            mailbox.respond(Response {
                id: run.request.id,
                result: Ok(run.results),
            });
        }
        Err(reason) => {
            let run = state.0.take().expect("failed run");
            mailbox.respond(Response {
                id: run.request.id,
                result: Err(reason),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn benchmark_targets_stay_inside_the_existing_memory_budget() {
        let request = Request {
            id: 1,
            resolution: [1920, 1080],
            sample_budgets_ns: BTreeMap::from([(1, 10_000_000), (4, 7_000_000)]),
            memory_budget_bytes: assets::texture::policy::MIN_TERRAIN_BYTES,
            cancellation: Default::default(),
        };
        assert!(fits(&request, 1, 4096));
        assert!(!fits(&request, 4, 4096));
        assert!(!fits(&request, 1, 1024));
        assert!(!fits(
            &Request {
                resolution: [u32::MAX; 2],
                ..request
            },
            8,
            u32::MAX
        ));
    }
}
