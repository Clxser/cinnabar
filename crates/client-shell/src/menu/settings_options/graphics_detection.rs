//! Runs device tuning without blocking menu input or replacing manual preferences.

use bevy::{prelude::*, render::renderer::RenderAdapter, window::PrimaryWindow};
use launcher::menu::settings_options::graphics_detection::{self, BENCHMARK_LIMIT, Scores};
use render_api::BenchmarkCancellation;
use render_scene::graphics_benchmark::{GraphicsBenchmark, Request};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};

use crate::menu::MenuRuntime;

struct Run {
    id: u64,
    key: String,
    started: Instant,
    cancellation: BenchmarkCancellation,
    cpu: crossbeam_channel::Receiver<u32>,
    cpu_score: Option<u32>,
    gpu_score: Option<BTreeMap<u32, u64>>,
    on_battery: bool,
    memory_budget: usize,
    explicit: bool,
}

impl Drop for Run {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

/// Owns one live run and its pending completion notice.
#[derive(Resource, Default)]
pub struct Detection {
    initialized: bool,
    next_id: u64,
    run: Option<Run>,
    notice: Option<String>,
}

/// Hashes the selected adapter and driver together with all measurement inputs.
fn cache_key(
    info: &wgpu::AdapterInfo,
    build: &str,
    resolution: [u32; 2],
    samples: &[u32],
    memory: u64,
) -> String {
    let identity = format!(
        "{}|{}|{}|{}|{:?}|{}|{}|{}|{:?}|{:?}|{}",
        graphics_detection::BENCHMARK_VERSION,
        info.vendor,
        info.device,
        info.name,
        info.backend,
        info.driver,
        info.driver_info,
        build,
        resolution,
        samples,
        memory
    );
    format!("{:x}", Sha256::digest(identity.as_bytes()))
}

/// Measures one core for 300 ms, checking cancellation before each bounded section.
fn cpu_worker(cancellation: BenchmarkCancellation) -> crossbeam_channel::Receiver<u32> {
    let (sender, receiver) = crossbeam_channel::bounded(1);
    std::thread::spawn(move || {
        if cancellation.is_cancelled() {
            return;
        }
        let mut workload = chunk_pipeline::graphics_workload::GraphicsWorkload::default();
        workload.run_section();
        let started = Instant::now();
        let mut sections = 0_u32;
        while started.elapsed() < Duration::from_millis(300) {
            if cancellation.is_cancelled() {
                return;
            }
            workload.run_section();
            sections += 1;
        }
        if !cancellation.is_cancelled() {
            let rate = (f64::from(sections) / started.elapsed().as_secs_f64()) as u32;
            let _ = sender.send(rate);
        }
    });
    receiver
}

impl MenuRuntime {
    /// Applies only automatic defaults and schedules the ordinary settings handoff.
    fn tune_graphics(&mut self, scores: &Scores, memory_budget: usize, on_battery: bool) -> bool {
        let recommendation = graphics_detection::recommend(
            &self.settings_options,
            scores,
            memory_budget,
            on_battery,
        );
        if Arc::make_mut(&mut self.settings_options).apply_graphics_recommendation(recommendation) {
            self.settings_dirty = true;
            self.settings_apply = true;
            return true;
        }
        false
    }
}

/// Starts after adapter selection, polls asynchronous results, and expires cancelled runs.
pub fn drive(
    mut state: ResMut<Detection>,
    adapter: Option<Res<RenderAdapter>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mailbox: Res<GraphicsBenchmark>,
    mut menu: ResMut<MenuRuntime>,
    mut ui: Option<ResMut<crate::ui_resources::UiRuntimeResource>>,
    time: Option<Res<Time<Real>>>,
) {
    let now = Instant::now();
    if let (Some(ui), Some(time)) = (ui.as_mut(), time.as_deref())
        && let Some(notice) = state.notice.take()
    {
        ui.push_client_toast(notice, time.elapsed().as_millis() as u64);
    }
    let explicit = std::mem::take(&mut menu.graphics_detection_requested);
    if explicit && state.run.take().is_some() {
        menu.graphics_detection_running = false;
        return;
    }
    // Leave a session join responsive, and never benchmark over live gameplay.
    if state.run.is_some() && (menu.is_connecting() || !menu.is_visible()) {
        state.run = None;
        menu.graphics_detection_running = false;
        return;
    }
    if let Some(run) = state.run.as_mut() {
        if now.duration_since(run.started) >= BENCHMARK_LIMIT {
            if run.explicit {
                state.notice =
                    Some("Graphics detection timed out. Your settings were kept.".to_owned());
            }
            state.run = None;
            menu.graphics_detection_running = false;
            return;
        }
        if let Ok(score) = run.cpu.try_recv() {
            run.cpu_score = Some(score);
        }
        if let Some(response) = mailbox.take_response()
            && response.id == run.id
        {
            match response.result {
                Ok(score) => run.gpu_score = Some(score),
                Err(reason) => {
                    if run.explicit {
                        state.notice = Some(format!(
                            "Graphics detection unavailable: {reason}. Your settings were kept."
                        ));
                    }
                    state.run = None;
                    menu.graphics_detection_running = false;
                    return;
                }
            }
        }
        if let Some(cpu) = run.cpu_score
            && let Some(gpu) = run.gpu_score.take()
        {
            let scores = Scores {
                sections_per_second: cpu,
                gpu_p99_ns: gpu,
                duration_ms: now.duration_since(run.started).as_millis().max(1) as u32,
            };
            let cached = Arc::make_mut(&mut menu.settings_options)
                .cache_graphics_scores(run.key.clone(), scores.clone());
            if cached {
                let tuned = menu.tune_graphics(&scores, run.memory_budget, run.on_battery);
                menu.settings_dirty = true;
                if tuned {
                    state.notice = Some(graphics_detection::TUNED_MESSAGE.to_owned());
                }
                info!(
                    duration_ms = scores.duration_ms,
                    "graphics benchmark completed"
                );
            }
            state.run = None;
            menu.graphics_detection_running = false;
        }
        return;
    }
    if state.initialized && !explicit {
        return;
    }
    let Some(adapter) = adapter else {
        return;
    };
    let Ok(window) = windows.single() else {
        return;
    };
    if window.resolution.physical_width() == 0
        || window.resolution.physical_height() == 0
        || menu.is_connecting()
        || !menu.is_visible()
    {
        return;
    }
    let resolution = [
        window.resolution.physical_width(),
        window.resolution.physical_height(),
    ];
    let sample_budgets = graphics_detection::sample_budgets(&menu.settings_options);
    let samples: Vec<_> = sample_budgets.keys().copied().collect();
    let memory = launcher_host::memory::physical_bytes();
    let tier = resource_pack::subpacks::device_memory_tier(memory);
    let budget = assets::texture::policy::TexturePolicy::for_terrain(
        adapter.limits().max_texture_dimension_2d,
        tier,
    )
    .storage_bytes();
    let on_battery = launcher_host::power::on_battery();
    let key = cache_key(
        &adapter.get_info(),
        &launcher_host::power::graphics_driver_build(),
        resolution,
        &samples,
        memory,
    );
    state.initialized = true;
    if !explicit && let Some(scores) = menu.settings_options.graphics_scores(&key).cloned() {
        if menu.tune_graphics(&scores, budget, on_battery) {
            state.notice = Some(graphics_detection::TUNED_MESSAGE.to_owned());
        }
        return;
    }
    state.next_id += 1;
    let cancellation = BenchmarkCancellation::default();
    let cpu = cpu_worker(cancellation.clone());
    mailbox.request(Request {
        id: state.next_id,
        resolution,
        sample_budgets_ns: sample_budgets,
        memory_budget_bytes: budget,
        cancellation: cancellation.clone(),
    });
    state.run = Some(Run {
        id: state.next_id,
        key,
        started: now,
        cancellation,
        cpu,
        cpu_score: None,
        gpu_score: None,
        on_battery,
        memory_budget: budget,
        explicit,
    });
    menu.graphics_detection_running = true;
}

#[cfg(test)]
mod tests;
