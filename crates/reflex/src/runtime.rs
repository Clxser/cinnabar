//! Shared simulation and render frame controller.

use std::{
    num::NonZeroU64,
    sync::{Arc, Mutex, MutexGuard},
};

use crate::{AdapterIdentity, Backend, Capabilities, FrameId, Marker, Mode, NoopBackend};

struct Controller {
    requested_mode: Mode,
    effective_mode: Mode,
    capabilities: Capabilities,
    next_frame: NonZeroU64,
    current_frame: Option<FrameId>,
    render_frame: Option<FrameId>,
    backend: Arc<dyn Backend>,
}

impl Controller {
    fn new(mode: Mode, capabilities: Capabilities, backend: Box<dyn Backend>) -> Self {
        let requested_effective_mode = if mode.is_enabled() && capabilities.ready() {
            mode
        } else {
            Mode::Off
        };
        let backend_ready = backend
            .set_latency_sleep_mode(requested_effective_mode)
            .is_ok();
        let effective_mode = if backend_ready {
            requested_effective_mode
        } else {
            Mode::Off
        };
        Self {
            requested_mode: mode,
            effective_mode,
            capabilities,
            next_frame: NonZeroU64::MIN,
            current_frame: None,
            render_frame: None,
            backend: backend.into(),
        }
    }

    fn begin_frame(&mut self) -> FrameId {
        let frame = FrameId(self.next_frame);
        self.next_frame =
            NonZeroU64::new(self.next_frame.get().saturating_add(1)).unwrap_or(NonZeroU64::MIN);
        self.current_frame = Some(frame);
        frame
    }
}

/// Cloneable main/render-world handle for the frame latency controller.
#[derive(Clone)]
pub struct Runtime(Arc<Mutex<Controller>>);

impl Runtime {
    /// Creates the safe fallback controller from an adapter report.
    #[must_use]
    pub fn new(mode: Mode, adapter: AdapterIdentity) -> Self {
        Self::with_backend(
            mode,
            Capabilities::detect(adapter),
            Box::<NoopBackend>::default(),
        )
    }

    /// Creates a controller around a native backend supplied by the renderer.
    #[must_use]
    pub fn with_backend(mode: Mode, capabilities: Capabilities, backend: Box<dyn Backend>) -> Self {
        Self(Arc::new(Mutex::new(Controller::new(
            mode,
            capabilities,
            backend,
        ))))
    }

    /// Replaces the conservative startup backend once the renderer has created its device.
    ///
    /// Device and swapchain capability detection must happen in the render world, while input
    /// admission happens in the main world; both worlds share this controller.
    pub fn install_backend(&self, capabilities: Capabilities, backend: Box<dyn Backend>) {
        let mut controller = self.guard();
        controller.capabilities = capabilities;
        let requested_effective_mode =
            if controller.requested_mode.is_enabled() && capabilities.ready() {
                controller.requested_mode
            } else {
                Mode::Off
            };
        let backend_ready = backend
            .set_latency_sleep_mode(requested_effective_mode)
            .is_ok();
        controller.effective_mode = if backend_ready {
            requested_effective_mode
        } else {
            Mode::Off
        };
        controller.backend = backend.into();
    }

    /// Changes the requested policy after startup, such as when the Video settings menu applies.
    /// The native backend updates its sleep mode without rebuilding the swapchain.
    pub fn set_requested_mode(&self, mode: Mode) {
        let mut controller = self.guard();
        if controller.requested_mode == mode {
            return;
        }
        controller.requested_mode = mode;
        let requested_effective_mode = if mode.is_enabled() && controller.capabilities.ready() {
            mode
        } else {
            Mode::Off
        };
        let backend_ready = controller
            .backend
            .set_latency_sleep_mode(requested_effective_mode)
            .is_ok();
        controller.effective_mode = if backend_ready {
            requested_effective_mode
        } else {
            Mode::Off
        };
    }

    fn guard(&self) -> MutexGuard<'_, Controller> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[must_use]
    pub fn requested_mode(&self) -> Mode {
        self.guard().requested_mode
    }

    #[must_use]
    pub fn effective_mode(&self) -> Mode {
        self.guard().effective_mode
    }

    #[must_use]
    pub fn capabilities(&self) -> Capabilities {
        self.guard().capabilities
    }

    pub fn begin_frame(&self) -> FrameId {
        self.guard().begin_frame()
    }

    /// Waits without holding the controller lock, so the previous render can finish presenting.
    /// A failed native wait disables pacing and markers until a mode change succeeds.
    pub fn sleep_before_input(&self, frame: FrameId) {
        let Some(backend) = self.active_backend() else {
            return;
        };
        if backend.latency_sleep(frame).is_err() {
            let mut controller = self.guard();
            if Arc::ptr_eq(&controller.backend, &backend) {
                controller.effective_mode = Mode::Off;
                let _ = backend.set_latency_sleep_mode(Mode::Off);
            }
        }
    }

    /// Marks the input gathered for this simulation, after pacing has completed.
    pub fn mark_input_sample(&self, frame: FrameId) {
        self.mark(frame, Marker::InputSample);
    }

    pub fn mark_simulation_start(&self, frame: FrameId) {
        self.mark(frame, Marker::SimulationStart);
    }

    pub fn mark_simulation_end(&self) {
        let frame = self.guard().current_frame;
        if let Some(frame) = frame {
            self.mark(frame, Marker::SimulationEnd);
        }
    }

    /// Associates rendering with the frame actually extracted, including skipped updates.
    pub fn begin_render(&self, frame: FrameId) {
        self.guard().render_frame = Some(frame);
    }

    pub fn mark_render_start(&self, frame: FrameId) {
        self.mark(frame, Marker::SubmissionStart);
    }

    pub fn mark_render_end(&self) {
        let frame = self.guard().render_frame.take();
        if let Some(frame) = frame {
            self.mark(frame, Marker::SubmissionEnd);
            self.mark(frame, Marker::PresentationStart);
            self.mark(frame, Marker::PresentationEnd);
        }
    }

    fn active_backend(&self) -> Option<Arc<dyn Backend>> {
        let controller = self.guard();
        controller
            .effective_mode
            .is_enabled()
            .then(|| controller.backend.clone())
    }

    fn mark(&self, frame: FrameId, marker: Marker) {
        if let Some(backend) = self.active_backend() {
            backend.set_latency_marker(frame, marker);
        }
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
