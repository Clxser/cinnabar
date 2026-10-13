use super::*;

#[test]
fn driver_resolution_and_workload_inputs_invalidate_the_cache() {
    let info = wgpu::AdapterInfo {
        name: "fixture".into(),
        vendor: 1,
        device: 2,
        device_type: wgpu::DeviceType::DiscreteGpu,
        device_pci_bus_id: String::new(),
        driver: "driver".into(),
        driver_info: "1".into(),
        backend: wgpu::Backend::Vulkan,
        subgroup_min_size: 1,
        subgroup_max_size: 1,
        transient_saves_memory: None,
        limit_bucket: None,
    };
    let key = cache_key(&info, "", [1280, 720], &[1, 4], 8 << 30);
    let changed = wgpu::AdapterInfo {
        driver_info: "2".into(),
        ..info.clone()
    };
    assert_ne!(key, cache_key(&changed, "", [1280, 720], &[1, 4], 8 << 30));
    assert_ne!(
        key,
        cache_key(&info, "new-os-build", [1280, 720], &[1, 4], 8 << 30)
    );
    assert_ne!(key, cache_key(&info, "", [1920, 1080], &[1, 4], 8 << 30));
    assert_ne!(key, cache_key(&info, "", [1280, 720], &[1], 8 << 30));
}

#[test]
fn cancelling_discards_late_results_without_saving_or_applying_them() {
    let mut app = App::new();
    let mut menu = MenuRuntime::new(true, 2, "Graphics test".into());
    menu.set_named_option("render_distance", 7);
    let settings = menu.settings_options.clone();
    menu.graphics_detection_running = true;
    menu.activate(launcher::menu::MenuAction::SettingsDetectGraphics);
    let cancellation = BenchmarkCancellation::default();
    let (sender, receiver) = crossbeam_channel::bounded(1);
    let key = format!("{:064x}", 1);
    app.insert_resource(menu)
        .insert_resource(Detection {
            initialized: true,
            next_id: 1,
            notice: None,
            run: Some(Run {
                id: 1,
                key: key.clone(),
                started: Instant::now(),
                cancellation: cancellation.clone(),
                cpu: receiver,
                cpu_score: None,
                gpu_score: None,
                on_battery: false,
                memory_budget: assets::texture::policy::MAX_TERRAIN_BYTES,
                explicit: true,
            }),
        })
        .init_resource::<GraphicsBenchmark>()
        .add_systems(Update, drive);
    app.update();
    assert!(cancellation.is_cancelled());
    assert!(sender.send(1_000).is_err());
    app.world().resource::<GraphicsBenchmark>().respond(
        render_scene::graphics_benchmark::Response {
            id: 1,
            result: Ok(BTreeMap::from([(1, 1_000_000)])),
        },
    );
    app.update();
    let menu = app.world().resource::<MenuRuntime>();
    assert!(!menu.graphics_detection_running);
    assert_eq!(*menu.settings_options, *settings);
    assert!(menu.settings_options.graphics_scores(&key).is_none());
}
