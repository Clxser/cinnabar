use super::*;
use std::sync::{
    Weak,
    atomic::{AtomicBool, Ordering},
};

#[derive(Debug, PartialEq)]
enum Call {
    Mode(Mode),
    Sleep(FrameId),
    Marker(FrameId, Marker),
}

#[derive(Default)]
struct Recorder {
    calls: Mutex<Vec<Call>>,
    controller: Mutex<Option<Weak<Mutex<Controller>>>>,
    fail_sleep: AtomicBool,
}

struct RecordingBackend(Arc<Recorder>);

impl Backend for RecordingBackend {
    fn set_latency_sleep_mode(&self, mode: Mode) -> Result<(), BackendError> {
        self.0.calls.lock().unwrap().push(Call::Mode(mode));
        Ok(())
    }

    fn latency_sleep(&self, frame: FrameId) -> Result<(), BackendError> {
        if let Some(controller) = self
            .0
            .controller
            .lock()
            .unwrap()
            .as_ref()
            .and_then(Weak::upgrade)
        {
            assert!(
                controller.try_lock().is_ok(),
                "the previous render cannot finish while input is waiting"
            );
        }
        self.0.calls.lock().unwrap().push(Call::Sleep(frame));
        if self.0.fail_sleep.load(Ordering::Relaxed) {
            Err(BackendError)
        } else {
            Ok(())
        }
    }

    fn set_latency_marker(&self, frame: FrameId, marker: Marker) {
        self.0
            .calls
            .lock()
            .unwrap()
            .push(Call::Marker(frame, marker));
    }
}

fn runtime(mode: Mode) -> (Runtime, Arc<Recorder>) {
    let recorder = Arc::<Recorder>::default();
    let runtime = Runtime::with_backend(
        mode,
        Capabilities::detect(AdapterIdentity::nvidia_vulkan(true, true)),
        Box::new(RecordingBackend(recorder.clone())),
    );
    *recorder.controller.lock().unwrap() = Some(Arc::downgrade(&runtime.0));
    (runtime, recorder)
}

fn simulate(runtime: &Runtime) -> FrameId {
    let frame = runtime.begin_frame();
    runtime.sleep_before_input(frame);
    runtime.mark_simulation_start(frame);
    runtime.mark_input_sample(frame);
    runtime.mark_simulation_end();
    frame
}

#[test]
fn off_issues_no_sleep_or_frame_markers() {
    let (runtime, recorder) = runtime(Mode::Off);
    let frame = simulate(&runtime);
    runtime.begin_render(frame);
    runtime.mark_render_start(frame);
    runtime.mark_render_end();
    assert_eq!(*recorder.calls.lock().unwrap(), [Call::Mode(Mode::Off)]);
}

#[test]
fn boost_transition_and_off_reach_the_native_policy() {
    let (runtime, recorder) = runtime(Mode::Off);
    runtime.set_requested_mode(Mode::On);
    let on = simulate(&runtime);
    runtime.set_requested_mode(Mode::OnBoost);
    let boost = simulate(&runtime);
    runtime.set_requested_mode(Mode::Off);
    simulate(&runtime);
    let calls = recorder.calls.lock().unwrap();
    let modes: Vec<_> = calls
        .iter()
        .filter_map(|call| match call {
            Call::Mode(mode) => Some(*mode),
            _ => None,
        })
        .collect();
    let sleeps: Vec<_> = calls
        .iter()
        .filter_map(|call| match call {
            Call::Sleep(frame) => Some(*frame),
            _ => None,
        })
        .collect();
    assert_eq!(modes, [Mode::Off, Mode::On, Mode::OnBoost, Mode::Off]);
    assert_eq!(sleeps, [on, boost]);
    assert_eq!(runtime.effective_mode(), Mode::Off);
}

#[test]
fn sleep_does_not_lock_out_the_previous_render() {
    let (runtime, recorder) = runtime(Mode::OnBoost);
    let frame = simulate(&runtime);
    assert!(recorder.calls.lock().unwrap().contains(&Call::Sleep(frame)));
}

#[test]
fn failed_sleep_disables_native_work_and_preserves_requested_mode() {
    let (runtime, recorder) = runtime(Mode::OnBoost);
    recorder.fail_sleep.store(true, Ordering::Relaxed);
    let frame = simulate(&runtime);
    simulate(&runtime);
    assert_eq!(runtime.requested_mode(), Mode::OnBoost);
    assert_eq!(runtime.effective_mode(), Mode::Off);
    assert_eq!(
        *recorder.calls.lock().unwrap(),
        [
            Call::Mode(Mode::OnBoost),
            Call::Sleep(frame),
            Call::Mode(Mode::Off)
        ]
    );
}

#[test]
fn render_markers_follow_the_extracted_frame_after_skipped_updates() {
    let (runtime, recorder) = runtime(Mode::On);
    simulate(&runtime);
    simulate(&runtime);
    let extracted = simulate(&runtime);
    runtime.begin_render(extracted);
    // Main simulation can advance while the extracted frame renders.
    simulate(&runtime);
    runtime.mark_render_start(extracted);
    runtime.mark_render_end();
    let calls = recorder.calls.lock().unwrap();
    let rendered: Vec<_> = calls
        .iter()
        .filter_map(|call| match call {
            Call::Marker(
                frame,
                Marker::SubmissionStart
                | Marker::SubmissionEnd
                | Marker::PresentationStart
                | Marker::PresentationEnd,
            ) => Some(*frame),
            _ => None,
        })
        .collect();
    assert_eq!(rendered, [extracted; 4]);
    assert!(calls.contains(&Call::Marker(extracted, Marker::InputSample)));
}

#[test]
fn unsupported_adapter_never_runs_native_frame_calls() {
    let recorder = Arc::<Recorder>::default();
    let runtime = Runtime::with_backend(
        Mode::OnBoost,
        Capabilities::detect(AdapterIdentity::unknown()),
        Box::new(RecordingBackend(recorder.clone())),
    );
    simulate(&runtime);
    assert_eq!(runtime.effective_mode(), Mode::Off);
    assert_eq!(*recorder.calls.lock().unwrap(), [Call::Mode(Mode::Off)]);
}
