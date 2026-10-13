//! Exercises the benchmark through a native offscreen render app when an adapter is available.

use super::*;
use crate::chunk::gpu_cull::app_tests::{chunk_app, frame, render_plugin};
use render_scene::graphics_benchmark::Request;
use std::{collections::BTreeMap, time::Instant};

#[test]
fn native_graphics_benchmark_returns_measured_terrain_timestamps() {
    let features = wgpu::Features::TIMESTAMP_QUERY;
    let Some(render) = render_plugin(wgpu::Backends::PRIMARY, features) else {
        eprintln!("missing fixture: native GPU with terrain timestamp queries");
        return;
    };
    let (mut app, _) = chunk_app(render, Msaa::Off, Transform::default());
    let mailbox = app.world().resource::<GraphicsBenchmark>().clone();
    let started = Instant::now();
    mailbox.request(Request {
        id: 1,
        resolution: [1280, 720],
        sample_budgets_ns: BTreeMap::from([(1, 10_000_000)]),
        memory_budget_bytes: assets::texture::policy::MIN_TERRAIN_BYTES,
        cancellation: Default::default(),
    });
    let mut response = None;
    test_time::eventually("completed terrain timestamp measurements", || {
        frame(&mut app);
        response = mailbox.take_response();
        response.is_some()
    });
    let response = response.unwrap();
    assert_eq!(response.id, 1);
    let scores = response.result.expect("valid terrain timestamps");
    assert!(scores[&1] > 0);
    eprintln!(
        "terrain benchmark: {:?}, GPU p99 {} ns",
        started.elapsed(),
        scores[&1]
    );
}
