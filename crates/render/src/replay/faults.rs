//! One device callback owner keeps failures observable after a secondary renderer closes.

use std::sync::{Arc, Mutex};

use bevy::{
    prelude::*,
    render::{
        error_handler::{RenderError, RenderErrorPolicy},
        renderer::RenderDevice,
    },
};

#[derive(Clone, Default, Resource)]
pub(super) struct DeviceFaults(Arc<Mutex<Option<String>>>);

pub(super) struct DeviceCallbackGuard {
    device: RenderDevice,
    faults: DeviceFaults,
}

impl DeviceCallbackGuard {
    /// Restores shared routing even if a secondary plugin panics while registering its own callbacks.
    pub(super) fn new(device: &RenderDevice, faults: &DeviceFaults) -> Self {
        Self {
            device: device.clone(),
            faults: faults.clone(),
        }
    }
}

impl Drop for DeviceCallbackGuard {
    fn drop(&mut self) {
        self.faults.install(&self.device);
    }
}

impl DeviceFaults {
    /// Installs a shared router after Bevy initializes a secondary manual renderer on this device.
    pub(super) fn install(&self, device: &RenderDevice) {
        let lost = self.clone();
        device
            .wgpu_device()
            .set_device_lost_callback(move |reason, description| {
                lost.record(format!("GPU device lost: {reason:?}: {description}"));
            });
        let uncaptured = self.clone();
        device
            .wgpu_device()
            .on_uncaptured_error(Arc::new(move |error| {
                uncaptured.record(format!("GPU rendering failed: {error}"));
            }));
    }

    /// The first device failure is the useful cause; later submission errors do not replace it.
    fn record(&self, error: String) {
        let mut fault = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if fault.is_none() {
            error!("{error}");
            *fault = Some(error);
        }
    }

    /// The UI/input thread does not wait on a driver callback's fault publication.
    pub(super) fn error(&self) -> Option<String> {
        self.0.try_lock().ok()?.clone()
    }
}

/// Preserves the application's fatal GPU-error behavior even while the preview worker is idle/closed.
pub(super) fn exit_on_fault(
    faults: Res<DeviceFaults>,
    mut exits: MessageWriter<AppExit>,
    mut reported: Local<bool>,
) {
    if !*reported && faults.error().is_some() {
        *reported = true;
        exits.write(AppExit::error());
    }
}

/// Errors caught during secondary initialization also reach the primary application's shutdown path.
pub(super) fn initialization_error(
    error: &RenderError,
    main: &mut World,
    _render: &mut World,
) -> RenderErrorPolicy {
    main.resource::<DeviceFaults>().record(format!(
        "GPU replay initialization failed: {:?}: {}",
        error.ty, error.description
    ));
    RenderErrorPolicy::StopRendering
}
