//! Developer VRAM attribution: `CINNABAR_GPU_MEMORY_REPORT=<seconds>` logs the device
//! allocator's totals and its largest allocations, summed by label, at that interval.
//! Backends without an allocator report (Metal, GL) log nothing.

use bevy::prelude::{IntoScheduleConfigs, Res, ResMut, Resource, SubApp, info};
use bevy::render::{Render, RenderSystems, renderer::RenderDevice};
use std::collections::HashMap;
use std::time::{Duration, Instant};

const ENV: &str = "CINNABAR_GPU_MEMORY_REPORT";
/// Labels listed per report, largest first.
const LISTED_LABELS: usize = 24;

#[derive(Resource)]
struct MemoryReport {
    interval: Duration,
    last: Option<Instant>,
}

/// The report interval, when requested.
fn requested() -> Option<Duration> {
    std::env::var(ENV)
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
        .map(Duration::from_secs_f64)
}

pub(super) fn install(render_app: &mut SubApp) {
    let Some(interval) = requested() else {
        return;
    };
    render_app
        .insert_resource(MemoryReport {
            interval,
            last: None,
        })
        .add_systems(Render, report.in_set(RenderSystems::Cleanup));
}

fn report(mut state: ResMut<MemoryReport>, device: Res<RenderDevice>) {
    let now = Instant::now();
    if state
        .last
        .is_some_and(|last| now.duration_since(last) < state.interval)
    {
        return;
    }
    state.last = Some(now);
    let Some(report) = device.wgpu_device().generate_allocator_report() else {
        return;
    };
    let mut by_label: HashMap<&str, (u64, u32)> = HashMap::new();
    for allocation in &report.allocations {
        let entry = by_label.entry(allocation.name.as_str()).or_default();
        entry.0 += allocation.size;
        entry.1 += 1;
    }
    let mut labels: Vec<_> = by_label.into_iter().collect();
    labels.sort_unstable_by_key(|(_, (bytes, _))| std::cmp::Reverse(*bytes));
    let listed: Vec<String> = labels
        .iter()
        .take(LISTED_LABELS)
        .map(|(label, (bytes, count))| format!("{label}={}MiB/{count}", bytes >> 20))
        .collect();
    info!(
        target: "cinnabar::gpu_memory",
        allocated_mib = report.total_allocated_bytes >> 20,
        reserved_mib = report.total_reserved_bytes >> 20,
        allocations = report.allocations.len(),
        "GPU memory: {}",
        listed.join(", ")
    );
}
