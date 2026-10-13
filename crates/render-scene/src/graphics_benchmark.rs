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
}

/// Extraction clones the mailbox, so callbacks cannot write directly into menu state.
#[derive(Resource, Clone, Default, ExtractResource)]
#[extract_app(RenderApp)]
pub struct GraphicsBenchmark(Arc<Mutex<Mailbox>>);

impl GraphicsBenchmark {
    /// Replaces the pending run, cancelling all workers of its predecessor.
    pub fn request(&self, request: Request) {
        let mut mailbox = self.0.lock().expect("graphics benchmark mailbox");
        if let Some(previous) = mailbox.request.replace(request) {
            previous.cancellation.cancel();
        }
        mailbox.response = None;
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
