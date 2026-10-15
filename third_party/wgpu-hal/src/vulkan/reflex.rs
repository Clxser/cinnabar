//! Native low-latency calls and their device/swapchain lifetime.

use ash::vk;

use super::{Device, DeviceShared};

/// Shared policy and reusable timeline state. Never hold this mutex during a host wait:
/// the preceding render must be able to submit and present while input admission waits.
#[derive(Default)]
pub(super) struct ReflexNativeState {
    enabled: bool,
    boost: bool,
    failed: bool,
    semaphore: Option<vk::Semaphore>,
    signal_value: u64,
    pending_value: Option<u64>,
    present_serial: u64,
    slept_present: Option<(vk::SwapchainKHR, u64)>,
    active_frame: u64,
    submission_started: u64,
    explicit_submission_active: bool,
}

impl Device {
    /// Reports whether this device has a usable native low-latency backend.
    pub fn reflex_low_latency_enabled(&self) -> bool {
        self.shared.reflex_enabled() && !self.shared.reflex.lock().failed
    }

    /// Applies Off, On or On + Boost without changing swapchain ownership.
    pub fn reflex_set_latency_mode(&self, enabled: bool, boost: bool) -> bool {
        if !self.shared.reflex_enabled() {
            return false;
        }
        let swapchains = self.shared.reflex_swapchains.read();
        let mut state = self.shared.reflex.lock();
        if state.failed {
            state.enabled = false;
            state.boost = false;
            state.active_frame = 0;
            return !enabled;
        }
        state.enabled = enabled;
        state.boost = enabled && boost;
        if !enabled {
            state.active_frame = 0;
        }
        let extension = self.shared.reflex_extension();
        let mode = vk::LatencySleepModeInfoNV::default()
            .low_latency_mode(enabled)
            .low_latency_boost(state.boost);
        for &swapchain in swapchains.iter() {
            // SAFETY: the read guard prevents retirement; None disables both mode and boost.
            if let Err(error) =
                unsafe { extension.set_latency_sleep_mode(swapchain, enabled.then_some(&mode)) }
            {
                state.failed = true;
                state.enabled = false;
                log::error!("vkSetLatencySleepModeNV failed: {error:?}");
                return false;
            }
        }
        true
    }

    /// Selects the ID of the simulation actually extracted into this render frame.
    pub fn reflex_set_active_frame(&self, frame_id: u64) {
        let mut state = self.shared.reflex.lock();
        state.active_frame = if state.enabled && !state.failed {
            frame_id
        } else {
            0
        };
    }

    /// Clears attribution after render cleanup, including frames with no presentation.
    pub fn reflex_finish_frame(&self) {
        self.shared.reflex.lock().active_frame = 0;
    }

    /// Issues one sleep between primary-window presents, using a device-lifetime semaphore.
    /// The swapchain read guard spans the asynchronous signal, but no marker/policy mutex does.
    pub fn reflex_latency_sleep(&self, frame_id: u64) -> bool {
        if !self.shared.reflex_enabled() || frame_id == 0 {
            return true;
        }
        let swapchains = self.shared.reflex_swapchains.read();
        let Some(&swapchain) = swapchains.last() else {
            // Window preparation has not created a surface, or the window is suspended.
            return true;
        };
        let (semaphore, value) = {
            let mut state = self.shared.reflex.lock();
            if state.failed {
                return false;
            }
            if !state.enabled {
                return true;
            }
            let interval = (swapchain, state.present_serial);
            if state.slept_present == Some(interval) {
                return true;
            }
            if state.pending_value.is_some() {
                return false;
            }
            let semaphore = if let Some(semaphore) = state.semaphore {
                semaphore
            } else {
                let mut timeline = vk::SemaphoreTypeCreateInfo::default()
                    .semaphore_type(vk::SemaphoreType::TIMELINE)
                    .initial_value(0);
                let info = vk::SemaphoreCreateInfo::default().push_next(&mut timeline);
                // SAFETY: timelineSemaphore was queried and enabled before device creation.
                match unsafe { self.shared.raw.create_semaphore(&info, None) } {
                    Ok(semaphore) => {
                        state.semaphore = Some(semaphore);
                        semaphore
                    }
                    Err(error) => {
                        state.failed = true;
                        log::error!("Reflex timeline creation failed: {error:?}");
                        return false;
                    }
                }
            };
            state.signal_value += 1;
            let value = state.signal_value;
            state.slept_present = Some(interval);
            let sleep = vk::LatencySleepInfoNV::default()
                .signal_semaphore(semaphore)
                .value(value);
            // SAFETY: both handles stay live through the wait; calls are serialized here.
            if let Err(error) = unsafe {
                self.shared
                    .reflex_extension()
                    .latency_sleep(swapchain, &sleep)
            } {
                state.failed = true;
                log::error!("vkLatencySleepNV failed: {error:?}");
                return false;
            }
            state.pending_value = Some(value);
            (semaphore, value)
        };
        let result = self.shared.reflex_wait(semaphore, value);
        let mut state = self.shared.reflex.lock();
        match result {
            Ok(()) => {
                state.pending_value = None;
                true
            }
            Err(error) => {
                // Do not destroy/reuse a semaphore while the driver can still signal it.
                state.failed = true;
                state.enabled = false;
                state.active_frame = 0;
                log::error!("Reflex timeline wait failed: {error:?}; native pacing disabled for this device");
                false
            }
        }
    }

    /// Marks input sampling and simulation with the same ID as their extracted render frame.
    pub fn reflex_set_latency_marker(&self, frame_id: u64, marker: u32) {
        if !self.shared.reflex_enabled() || frame_id == 0 || !matches!(marker, 0 | 1 | 6) {
            return;
        }
        let swapchains = self.shared.reflex_swapchains.read();
        let state = self.shared.reflex.lock();
        if state.enabled && !state.failed {
            if let Some(&swapchain) = swapchains.last() {
                self.shared.reflex_marker(
                    swapchain,
                    frame_id,
                    vk::LatencyMarkerNV::from_raw(marker as i32),
                );
            }
        }
    }
}

impl DeviceShared {
    pub(super) fn reflex_enabled(&self) -> bool {
        self.enabled_extensions
            .contains(&ash::nv::low_latency2::NAME)
    }

    fn reflex_extension(&self) -> ash::nv::low_latency2::Device {
        ash::nv::low_latency2::Device::new(&self.instance.raw, &self.raw)
    }

    fn reflex_wait(&self, semaphore: vk::Semaphore, value: u64) -> Result<(), vk::Result> {
        let semaphores = [semaphore];
        let values = [value];
        let wait = vk::SemaphoreWaitInfo::default()
            .semaphores(&semaphores)
            .values(&values);
        // SAFETY: the semaphore is owned by this device and retained until shutdown.
        unsafe {
            match &self.extension_fns.timeline_semaphore {
                Some(super::ExtensionFn::Extension(extension)) => {
                    extension.wait_semaphores(&wait, 1_000_000_000)
                }
                Some(super::ExtensionFn::Promoted) => {
                    self.raw.wait_semaphores(&wait, 1_000_000_000)
                }
                None => Err(vk::Result::ERROR_FEATURE_NOT_PRESENT),
            }
        }
    }

    pub(super) fn reflex_register_swapchain(&self, swapchain: vk::SwapchainKHR) {
        if !self.reflex_enabled() {
            return;
        }
        let mut swapchains = self.reflex_swapchains.write();
        let mut state = self.reflex.lock();
        if state.failed {
            return;
        }
        let mode = vk::LatencySleepModeInfoNV::default()
            .low_latency_mode(state.enabled)
            .low_latency_boost(state.boost);
        // SAFETY: creation installed the latency pNext; registration owns the new live handle.
        if let Err(error) = unsafe {
            self.reflex_extension()
                .set_latency_sleep_mode(swapchain, state.enabled.then_some(&mode))
        } {
            state.failed = true;
            state.enabled = false;
            log::error!("Reflex swapchain configuration failed: {error:?}");
        }
        swapchains.push(swapchain);
        state.slept_present = None;
    }

    /// Waits for latency calls to release their lifetime guard before retiring this surface.
    pub(super) fn reflex_unregister_swapchain(&self, swapchain: vk::SwapchainKHR) {
        self.reflex_swapchains
            .write()
            .retain(|candidate| *candidate != swapchain);
    }

    fn reflex_marker(
        &self,
        swapchain: vk::SwapchainKHR,
        frame_id: u64,
        marker: vk::LatencyMarkerNV,
    ) {
        let info = vk::SetLatencyMarkerInfoNV::default()
            .present_id(frame_id)
            .marker(marker);
        // SAFETY: callers retain the registered swapchain and serialize extension calls.
        unsafe { self.reflex_extension().set_latency_marker(swapchain, &info) };
    }

    /// Captures attribution once for the complete queue submission and both of its markers.
    pub(super) fn reflex_begin_submission(&self) -> u64 {
        if !self.reflex_enabled() {
            return 0;
        }
        let swapchains = self.reflex_swapchains.read();
        let mut state = self.reflex.lock();
        let frame = state.active_frame;
        if !state.enabled || state.failed || frame == 0 {
            return 0;
        }
        if state.submission_started != frame {
            if let Some(&swapchain) = swapchains.last() {
                self.reflex_marker(swapchain, frame, vk::LatencyMarkerNV::RENDERSUBMIT_START);
            }
            state.submission_started = frame;
        }
        frame
    }

    pub(super) fn reflex_mark_submission_end(&self, frame: u64) {
        if frame == 0 {
            return;
        }
        let swapchains = self.reflex_swapchains.read();
        let state = self.reflex.lock();
        if state.enabled && !state.failed {
            if let Some(&swapchain) = swapchains.last() {
                self.reflex_marker(swapchain, frame, vk::LatencyMarkerNV::RENDERSUBMIT_END);
            }
        }
    }

    /// Starts explicit attribution only when supported, and clears it once on transition to Off.
    pub(super) fn reflex_submission_info(
        &self,
        frame: u64,
    ) -> Option<vk::LatencySubmissionPresentIdNV<'static>> {
        if !self.reflex_enabled() || self.reflex_extension_revision < 3 {
            return None;
        }
        let mut state = self.reflex.lock();
        if state.failed || (frame == 0 && !state.explicit_submission_active) {
            return None;
        }
        state.explicit_submission_active = frame != 0;
        Some(vk::LatencySubmissionPresentIdNV::default().present_id(frame))
    }

    /// Stops extension calls as soon as a queue or presentation reports device loss.
    pub(super) fn reflex_device_lost(&self) {
        let mut state = self.reflex.lock();
        state.failed = true;
        state.enabled = false;
        state.boost = false;
        state.active_frame = 0;
    }

    pub(super) fn reflex_begin_present(&self, swapchain: vk::SwapchainKHR) -> u64 {
        if !self.reflex_enabled() {
            return 0;
        }
        let swapchains = self.reflex_swapchains.read();
        let state = self.reflex.lock();
        if !state.enabled
            || state.failed
            || swapchains.last() != Some(&swapchain)
            || state.active_frame == 0
        {
            return 0;
        }
        self.reflex_marker(
            swapchain,
            state.active_frame,
            vk::LatencyMarkerNV::PRESENT_START,
        );
        state.active_frame
    }

    pub(super) fn reflex_end_present(
        &self,
        swapchain: vk::SwapchainKHR,
        frame: u64,
        device_lost: bool,
    ) {
        if !self.reflex_enabled() {
            return;
        }
        let swapchains = self.reflex_swapchains.read();
        let mut state = self.reflex.lock();
        if device_lost {
            state.failed = true;
            state.enabled = false;
            state.active_frame = 0;
        }
        if frame != 0 && state.enabled && !state.failed {
            self.reflex_marker(swapchain, frame, vk::LatencyMarkerNV::PRESENT_END);
        }
        if swapchains.last() == Some(&swapchain) {
            state.present_serial += 1;
        }
    }

    /// Frees the reusable semaphore only after its final signal has completed.
    pub(super) fn reflex_shutdown(&self) {
        let mut state = self.reflex.lock();
        if let Some(semaphore) = state.semaphore.take() {
            if let Some(value) = state.pending_value {
                match self.reflex_wait(semaphore, value) {
                    Ok(()) | Err(vk::Result::ERROR_DEVICE_LOST) => (),
                    Err(error) => {
                        log::error!("Reflex signal still pending at device shutdown: {error:?}");
                        return;
                    }
                }
            }
            // SAFETY: the last native signal completed, or device loss cancelled device work.
            unsafe { self.raw.destroy_semaphore(semaphore, None) };
        }
    }
}
