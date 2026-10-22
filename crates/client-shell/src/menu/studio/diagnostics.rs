//! A single cancellable diagnostic worker keeps capability probing off storage and UI threads.

use super::*;
use recording_studio::{
    export::Cancellation,
    model::{Encoder, ExportCodec},
    video::diagnostics,
};
use std::sync::{
    Mutex,
    mpsc::{Receiver, TryRecvError},
};

#[derive(Default)]
pub(super) struct Diagnostics {
    job: Option<Job>,
}
struct Job {
    cancellation: Cancellation,
    result: Mutex<Receiver<Result<diagnostics::Report, String>>>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

impl Studio {
    /// One explicit retry repeats tool and driver discovery; a second click cannot start another probe.
    pub(super) fn check_dependencies(&mut self) -> Result<(), String> {
        if self.diagnostics.job.is_some() {
            return Ok(());
        }
        if self.view.capture.phase.busy() || self.view.export.phase.busy() {
            return Err(
                "Finish recording/export before probing encoder devices, then retry.".into(),
            );
        }
        let codec = self
            .editor
            .as_ref()
            .map_or(ExportCodec::H264, |editor| editor.project().export.codec);
        let cancellation = Cancellation::default();
        let work = cancellation.clone();
        let (done, result) = std::sync::mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("studio-dependency-check".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    diagnostics::check(codec, &work)
                }))
                .unwrap_or_else(|_| {
                    Err("The dependency checker stopped unexpectedly. Retry the check.".into())
                });
                let _ = done.send(result);
            })
            .map_err(|error| error.to_string())?;
        self.diagnostics.job = Some(Job {
            cancellation,
            result: Mutex::new(result),
        });
        let view = &mut Arc::make_mut(&mut self.view).workflow.diagnostics;
        view.checking = true;
        view.error = None;
        Ok(())
    }

    /// Starting real output cancels a capability probe before the recorder/exporter acquires the device.
    pub(super) fn cancel_diagnostics(&mut self) {
        self.diagnostics.job = None;
        Arc::make_mut(&mut self.view).workflow.diagnostics.checking = false;
    }

    /// Results retain their checked codec so changing project settings cannot mislabel an old hardware result.
    pub(super) fn poll_diagnostics(&mut self) {
        let Some(job) = &mut self.diagnostics.job else {
            return;
        };
        let receiver = job
            .result
            .get_mut()
            .unwrap_or_else(|error| error.into_inner());
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                Err("The dependency checker stopped. Retry the check.".into())
            }
        };
        self.diagnostics.job = None;
        let view = &mut Arc::make_mut(&mut self.view).workflow.diagnostics;
        view.checking = false;
        match result {
            Ok(report) => {
                view.report = Some(Arc::new(report));
                view.error = None;
            }
            Err(error) => {
                view.report = None;
                view.error = Some(error);
            }
        }
    }

    /// CPU fallback is an explicit reversible export change and requires a matching successful software probe.
    pub(super) fn use_cpu_encoder(&mut self) -> Result<(), String> {
        if self.view.export.phase.busy() {
            return Err("Finish or cancel export before changing its encoder.".into());
        }
        let mut settings = self.project()?.export.clone();
        let report = self
            .view
            .workflow
            .diagnostics
            .report
            .as_ref()
            .ok_or("Check dependencies before choosing the verified CPU fallback.")?;
        if report.codec != settings.codec || !report.software.available {
            return Err("Run the dependency check for the current output format before choosing CPU encoding.".into());
        }
        settings.encoder = Encoder::Cpu;
        self.edit(
            "Use verified CPU export encoder",
            &[Command::SetExport { settings }],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_jobs_preserve_the_menu_resource_contract() {
        fn resource<T: bevy::prelude::Resource + Send + Sync>() {}
        resource::<crate::menu::MenuRuntime>();
    }

    #[test]
    fn pending_diagnostics_poll_without_waiting_and_publish_the_completed_error() {
        let mut studio = Studio::new("unused-diagnostic-test".into());
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        studio.diagnostics.job = Some(Job {
            cancellation: Cancellation::default(),
            result: Mutex::new(receiver),
        });
        Arc::make_mut(&mut studio.view)
            .workflow
            .diagnostics
            .checking = true;
        studio.poll_diagnostics();
        assert!(studio.view.workflow.diagnostics.checking);
        assert!(studio.diagnostics.job.is_some());

        sender.send(Err("test probe failed".into())).unwrap();
        studio.poll_diagnostics();
        assert!(!studio.view.workflow.diagnostics.checking);
        assert!(studio.diagnostics.job.is_none());
        assert_eq!(
            studio.view.workflow.diagnostics.error.as_deref(),
            Some("test probe failed")
        );
    }

    #[test]
    fn disconnected_diagnostics_clear_the_busy_state() {
        let mut studio = Studio::new("unused-diagnostic-test".into());
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        drop(sender);
        studio.diagnostics.job = Some(Job {
            cancellation: Cancellation::default(),
            result: Mutex::new(receiver),
        });
        Arc::make_mut(&mut studio.view)
            .workflow
            .diagnostics
            .checking = true;
        studio.poll_diagnostics();
        assert!(!studio.view.workflow.diagnostics.checking);
        assert!(studio.diagnostics.job.is_none());
        assert!(studio.view.workflow.diagnostics.error.is_some());
    }
}
