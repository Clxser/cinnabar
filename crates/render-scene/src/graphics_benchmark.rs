//! A bounded mailbox between the menu and the selected rendering device.

use bevy::{
    prelude::Resource,
    render::{RenderApp, extract_resource::ExtractResource},
};
use render_api::BenchmarkCancellation;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

/// Fixed terrain requested at the actual window resolution and supported sample counts.
#[derive(Clone)]
pub struct Request {
    pub id: u64,
    pub resolution: [u32; 2],
    pub sample_budgets_ns: BTreeMap<u32, u64>,
    pub memory_budget_bytes: usize,
    pub cancellation: BenchmarkCancellation,
}

/// Timestamp measurements, or a reason to retain the current settings.
pub struct Response {
    pub id: u64,
    pub result: Result<BTreeMap<u32, u64>, &'static str>,
}

#[derive(Default)]
struct Mailbox {
    request: Option<Request>,
    response: Option<Response>,
    cancellation: Option<BenchmarkCancellation>,
}

/// Extraction clones the mailbox, so callbacks cannot write directly into menu state.
#[derive(Resource, Clone, Default, ExtractResource)]
#[extract_app(RenderApp)]
pub struct GraphicsBenchmark(Arc<Mutex<Mailbox>>);

impl GraphicsBenchmark {
    /// Replaces the pending run, cancelling all workers of its predecessor.
    pub fn request(&self, request: Request) {
        let mut mailbox = self.0.lock().expect("graphics benchmark mailbox");
        if let Some(previous) = mailbox.cancellation.replace(request.cancellation.clone()) {
            previous.cancel();
        }
        mailbox.request = Some(request);
        mailbox.response = None;
    }

    /// Keeps temporary pacing active until the menu finishes or cancels both workers.
    pub fn is_running(&self) -> bool {
        self.0
            .lock()
            .expect("graphics benchmark mailbox")
            .cancellation
            .as_ref()
            .is_some_and(|cancellation| !cancellation.is_cancelled())
    }

    /// Takes a pending request exactly once on the rendering thread.
    pub fn take_request(&self) -> Option<Request> {
        self.0
            .lock()
            .expect("graphics benchmark mailbox")
            .request
            .take()
    }

    /// Publishes a completed GPU run for the menu to inspect on its next frame.
    pub fn respond(&self, response: Response) {
        self.0.lock().expect("graphics benchmark mailbox").response = Some(response);
    }

    /// Drains the one-slot result queue; the caller rejects stale run identities.
    pub fn take_response(&self) -> Option<Response> {
        self.0
            .lock()
            .expect("graphics benchmark mailbox")
            .response
            .take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacing_an_extracted_request_cancels_its_workers_and_retains_run_state() {
        let mailbox = GraphicsBenchmark::default();
        assert!(!mailbox.is_running());
        let request = Request {
            id: 1,
            resolution: [1280, 720],
            sample_budgets_ns: BTreeMap::from([(1, 10_000_000)]),
            memory_budget_bytes: assets::texture::policy::MIN_TERRAIN_BYTES,
            cancellation: Default::default(),
        };
        mailbox.request(request.clone());
        assert!(mailbox.take_request().is_some());
        assert!(mailbox.is_running());
        let replacement = Request {
            id: 2,
            cancellation: Default::default(),
            ..request.clone()
        };
        mailbox.request(replacement.clone());
        assert!(request.cancellation.is_cancelled());
        assert!(mailbox.is_running());
        replacement.cancellation.cancel();
        assert!(!mailbox.is_running());
    }
}
