//! A hidden window without a native surface still produces original camera pixels.

use std::sync::{Arc, Mutex};

use bevy::{
    asset::AssetPlugin,
    camera::CameraPlugin,
    core_pipeline::{CorePipelinePlugin, tonemapping::Tonemapping},
    mesh::MeshPlugin,
    prelude::*,
    render::{
        RenderApp, RenderPlugin,
        renderer::{RenderAdapter, RenderAdapterInfo, RenderDevice, RenderInstance, RenderQueue},
        settings::RenderCreation,
    },
    window::{PrimaryWindow, WindowPlugin, WindowResolution},
};

/// Uses a native offscreen device and names the missing fixture when none is available.
fn renderer() -> Option<RenderPlugin> {
    bevy::tasks::ComputeTaskPool::get_or_init(bevy::tasks::TaskPool::new);
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = match bevy::tasks::block_on(instance.request_adapter(&Default::default())) {
        Ok(adapter) => adapter,
        Err(wgpu::RequestAdapterError::NotFound { .. }) => {
            eprintln!(
                "skipping hidden_camera_pixels_reach_shared_capture: missing native GPU fixture"
            );
            return None;
        }
        Err(error) => panic!("capture fixture adapter: {error}"),
    };
    let info = adapter.get_info();
    let (device, queue) = bevy::tasks::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: adapter.limits(),
        ..default()
    }))
    .expect("capture fixture device");
    Some(RenderPlugin {
        render_creation: RenderCreation::manual(
            RenderDevice::from(device),
            RenderQueue::new(queue),
            RenderAdapterInfo::new(info),
            RenderAdapter::new(adapter),
            RenderInstance::new(instance),
        ),
        synchronous_pipeline_compilation: true,
        ..default()
    })
}

/// Captures through the same readback shared by screenshots and recordings.
fn capture(app: &mut App) -> Image {
    let result = Arc::new(Mutex::new(None));
    let received = result.clone();
    app.world_mut()
        .resource_mut::<super::FrameCapture>()
        .request(move |image| {
            *received.lock().unwrap() = Some(image.clone());
        });
    test_time::eventually("hidden camera pixel readback", || {
        app.update();
        app.sub_app(RenderApp)
            .world()
            .resource::<RenderDevice>()
            .wgpu_device()
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        result.lock().unwrap().is_some()
    });
    result.lock().unwrap().take().unwrap()
}

/// Checks rendered color and dimensions without relying on camera or schedule internals.
fn assert_blue(image: Image, expected_size: (u32, u32)) {
    let pixels = image.try_into_dynamic().unwrap().to_rgb8();
    assert_eq!(pixels.dimensions(), expected_size);
    let pixel = pixels.get_pixel(expected_size.0 / 2, expected_size.1 / 2).0;
    assert!(
        pixel[2] > 100 && pixel[2] > pixel[1] && pixel[1] > pixel[0],
        "blue camera output, got {pixel:?}"
    );
}

/// Builds the same hidden camera for display and clean-video admission checks.
fn app() -> Option<App> {
    let renderer = renderer()?;
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(WindowPlugin {
            primary_window: Some(Window {
                visible: false,
                focused: false,
                resolution: WindowResolution::new(64, 32).with_scale_factor_override(1.0),
                ..default()
            }),
            ..default()
        })
        .add_plugins(AssetPlugin::default())
        .add_plugins(renderer)
        .add_plugins((
            ImagePlugin::default(),
            MeshPlugin,
            CameraPlugin,
            CorePipelinePlugin,
        ));
    super::configure_frame_capture(&mut app);
    app.world_mut().spawn((
        Camera3d::default(),
        Camera {
            clear_color: ClearColorConfig::Custom(Color::srgb(0.2, 0.4, 0.8)),
            ..default()
        },
        client_presentation::camera::FlyCamera::default(),
        Msaa::Off,
        Tonemapping::Linear,
    ));
    Some(app)
}

#[test]
fn hidden_camera_pixels_reach_shared_capture() {
    let Some(mut app) = app() else { return };
    app.finish();
    app.cleanup();
    for _ in 0..8 {
        app.update();
    }
    assert_blue(capture(&mut app), (64, 32));
    {
        let mut windows = app
            .world_mut()
            .query_filtered::<&mut Window, With<PrimaryWindow>>();
        windows
            .single_mut(app.world_mut())
            .unwrap()
            .resolution
            .set_physical_resolution(32, 16);
    }
    for _ in 0..4 {
        app.update();
    }
    assert_blue(capture(&mut app), (32, 16));
}

/// Keeps requesting through the initial GPU asset admission, as recording frame slots do.
fn video(app: &mut App, size: [u32; 2], hud: bool) -> Result<Image, String> {
    let result = Arc::new(Mutex::new(None));
    test_time::eventually("clean recording pixel readback", || {
        let received = Arc::clone(&result);
        app.world_mut()
            .resource_mut::<super::FrameCapture>()
            .request_sized(size, [64, 32], hud, move |image| {
                let mut result = received.lock().unwrap();
                if result.is_none() {
                    *result = Some(image);
                }
            })
            .unwrap();
        app.update();
        app.sub_app(RenderApp)
            .world()
            .resource::<RenderDevice>()
            .wgpu_device()
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        result.lock().unwrap().is_some()
    });
    result.lock().unwrap().take().unwrap()
}

/// A solid screen layer makes a leaked HUD or an accidentally hidden live HUD observable.
fn screen_hud(
    view: bevy::render::renderer::ViewQuery<&bevy::render::view::ViewTarget>,
    mut context: bevy::render::renderer::RenderContext,
) {
    use bevy::render::render_resource::{LoadOp, RenderPassDescriptor};
    let mut color = view.into_inner().get_unsampled_color_attachment();
    color.ops.load = LoadOp::Clear(wgpu::Color::RED);
    let attachments = [Some(color)];
    let _pass = context
        .command_encoder()
        .begin_render_pass(&RenderPassDescriptor {
            label: Some("test screen HUD"),
            color_attachments: &attachments,
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
}

/// Draws an observable screen layer through the normal camera output after the clean sample.
fn install_hud(world: &mut World) {
    use bevy::core_pipeline::{Core3d, Core3dSystems, upscaling::upscaling};
    world.schedule_scope(Core3d, |_, schedule| {
        schedule.add_systems(
            screen_hud
                .after(Core3dSystems::PostProcess)
                .after(render::frame_capture::WorldFrameCapture)
                .before(upscaling),
        );
    });
}

#[test]
fn clean_video_preserves_world_pixels_and_live_hud_without_old_frame_fallback() {
    let Some(mut app) = app() else { return };
    app.sub_app_mut(RenderApp)
        .add_systems(bevy::render::RenderStartup, install_hud);
    app.finish();
    app.cleanup();
    for _ in 0..8 {
        app.update();
    }
    // Both bounded scale and Native size must sample before screen HUD composition.
    assert_blue(video(&mut app, [32, 16], false).unwrap(), (32, 16));
    assert_blue(video(&mut app, [64, 32], false).unwrap(), (64, 32));
    let display = capture(&mut app).try_into_dynamic().unwrap().to_rgb8();
    assert_eq!(display.get_pixel(32, 16).0, [255, 0, 0]);
    let recorded_hud = video(&mut app, [32, 16], true)
        .unwrap()
        .try_into_dynamic()
        .unwrap()
        .to_rgb8();
    assert_eq!(recorded_hud.get_pixel(16, 8).0, [255, 0, 0]);
    // Native dimensions with HUD enabled use the same reusable output and retain its exact pixels.
    let native_hud = video(&mut app, [64, 32], true)
        .unwrap()
        .try_into_dynamic()
        .unwrap()
        .to_rgb8();
    assert_eq!(native_hud.dimensions(), (64, 32));
    assert_eq!(native_hud.get_pixel(32, 16).0, [255, 0, 0]);

    let mut cameras = app.world_mut().query::<&mut Camera>();
    cameras.single_mut(app.world_mut()).unwrap().clear_color =
        ClearColorConfig::Custom(Color::srgb(0.1, 0.8, 0.2));
    let current = video(&mut app, [32, 16], false)
        .unwrap()
        .try_into_dynamic()
        .unwrap()
        .to_rgb8();
    let [red, green, blue] = current.get_pixel(16, 8).0;
    assert!(green > 100 && green > red && green > blue);

    // The reusable output still contains green pixels, but no current view may deliver those.
    cameras.single_mut(app.world_mut()).unwrap().is_active = false;
    assert!(video(&mut app, [32, 16], false).is_err());
}

#[test]
fn native_screenshot_keeps_priority_over_a_native_hud_recording_slot() {
    /// Cancellation releases the recording owner's pending slot without delivering an old image.
    struct Slot(Arc<Mutex<bool>>);
    impl Drop for Slot {
        fn drop(&mut self) {
            *self.0.lock().unwrap() = true;
        }
    }
    let Some(mut app) = app() else { return };
    app.sub_app_mut(RenderApp)
        .add_systems(bevy::render::RenderStartup, install_hud);
    app.finish();
    app.cleanup();
    for _ in 0..8 {
        app.update();
    }
    let released = Arc::new(Mutex::new(false));
    let slot = Slot(Arc::clone(&released));
    let result = Arc::new(Mutex::new(None));
    let received = Arc::clone(&result);
    {
        let mut requests = app.world_mut().resource_mut::<super::FrameCapture>();
        requests
            .request_sized([64, 32], [64, 32], true, move |_| {
                drop(slot);
                panic!("a screenshot must cancel this recording slot before capture");
            })
            .unwrap();
        requests.request_owned(move |image| {
            *received.lock().unwrap() = Some(image);
        });
    }
    test_time::eventually("native screenshot priority during recording", || {
        app.update();
        app.sub_app(RenderApp)
            .world()
            .resource::<RenderDevice>()
            .wgpu_device()
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        result.lock().unwrap().is_some()
    });
    assert!(*released.lock().unwrap());
    let screenshot = result
        .lock()
        .unwrap()
        .take()
        .unwrap()
        .try_into_dynamic()
        .unwrap()
        .to_rgb8();
    assert_eq!(screenshot.dimensions(), (64, 32));
    assert_eq!(screenshot.get_pixel(32, 16).0, [255, 0, 0]);
}
