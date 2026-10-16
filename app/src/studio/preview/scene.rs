//! Native replay sources use their own renderer; raster video never borrows the live world.

use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use recording_scene::{
    actors::ActorCoverage, playback::ScenePlayback, scene::DEFAULT_RESIDENT_BYTES,
};
use recording_studio::{
    Id, MediaTime,
    model::{Framing, MediaKind},
    playback::{CompiledTimeline, FramePlan},
    preview::{PreviewRequest, fit_size},
};
use render::replay::{ReplayRenderContext, ReplayRenderer};
use render_model::{UI_STUDIO_PREVIEW_MAX_SIZE, UiTexturePage};

use super::PreviewOutput;

pub(super) struct NativeScene {
    context: ReplayRenderContext,
    timeline: Option<CompiledTimeline>,
    plan: FramePlan,
    source: Option<Source>,
    placeholder: Option<UiTexturePage>,
}

struct Source {
    id: Id,
    path: PathBuf,
    document: u64,
    epoch: Option<[u64; 3]>,
    playback: ScenePlayback,
    renderer: ReplayRenderer,
}

impl NativeScene {
    /// Scene and GPU contexts are created only on the existing preview worker.
    pub(super) fn new(context: ReplayRenderContext) -> Self {
        Self {
            context,
            timeline: None,
            plan: FramePlan::default(),
            source: None,
            placeholder: None,
        }
    }

    /// Returns `None` for a raster-only frame; mixed replay composition remains explicitly unsupported.
    pub(super) fn render(
        &mut self,
        request: &PreviewRequest,
        cancelled: &impl Fn() -> bool,
        wait: &impl Fn(),
    ) -> Result<Option<PreviewOutput>, String> {
        if self
            .timeline
            .as_ref()
            .is_none_or(|timeline| !Arc::ptr_eq(timeline.project(), &request.project))
        {
            self.timeline = Some(
                CompiledTimeline::new(Arc::clone(&request.project))
                    .map_err(|error| error.to_string())?,
            );
        }
        let timeline = self
            .timeline
            .as_ref()
            .ok_or("The replay timeline is unavailable.")?;
        let at = MediaTime(
            request
                .at
                .0
                .min(request.project.duration().0.saturating_sub(1)),
        );
        timeline
            .sample(at, &mut self.plan, |_, _, _| None)
            .map_err(|error| error.to_string())?;
        if !self.plan.visuals.iter().any(|visual| {
            request
                .project
                .sources
                .iter()
                .any(|source| source.id == visual.source && source.kind == MediaKind::Replay)
        }) {
            self.source = None;
            return Ok(None);
        }
        let [visual] = &self.plan.visuals[..] else {
            return Err(
                "Combining a native replay with another visual layer is not connected yet.".into(),
            );
        };
        if visual.mix.is_some() || visual.framing != Framing::default() {
            return Err(
                "Native replay crops, opacity and transitions are not connected yet.".into(),
            );
        }
        let media = request
            .project
            .sources
            .iter()
            .find(|source| source.id == visual.source)
            .ok_or("The replay source is missing.")?;
        let path = if media.path.is_absolute() {
            media.path.clone()
        } else {
            request.source_base.join(&media.path)
        };
        let size = fit_size(request.project.export.size, UI_STUDIO_PREVIEW_MAX_SIZE)?;
        if self.source.as_ref().is_none_or(|source| {
            source.id != media.id
                || source.path != path
                || source.document != request.epoch[0]
                || source.renderer.size() != size
        }) {
            self.source = None;
            if cancelled() {
                return Err("Preview cancelled.".into());
            }
            let playback = ScenePlayback::open(&path, DEFAULT_RESIDENT_BYTES)
                .map_err(|error| format!("Cannot open native replay: {error}"))?;
            let renderer = ReplayRenderer::new(self.context.clone(), size)?;
            self.source = Some(Source {
                id: media.id,
                path,
                document: request.epoch[0],
                epoch: None,
                playback,
                renderer,
            });
        }
        let source = self
            .source
            .as_mut()
            .ok_or("The replay renderer is unavailable.")?;
        if visual.time >= source.playback.duration() {
            return Err(
                "This frame lies past the native scene tracks' common recorded duration.".into(),
            );
        }
        let frame = if source.epoch != Some(request.epoch)
            || source
                .playback
                .position()
                .is_none_or(|previous| previous > visual.time)
        {
            source.playback.seek(visual.time, cancelled)
        } else {
            source.playback.advance_to(visual.time, cancelled)
        }
        .map_err(|error| format!("Native replay admission failed: {error}"))?;
        source.epoch = Some(request.epoch);
        let pose = self.plan.camera.unwrap_or(frame.view.pose);
        source.renderer.apply(&frame, pose)?;
        let mut notices = Vec::new();
        notices.push(
            "3D preview uses Vanilla shading and omits HUD, hand, particles and block entities.",
        );
        if frame.actors.sample.coverage == ActorCoverage::OriginalView {
            notices.push(
                "Actors cover the recorded view only; other cameras can expose missing actors.",
            );
        }
        if self.plan.camera.is_none() && !request.project.shots.is_empty() {
            notices.push("Target camera tracking is not connected; showing the recorded camera.");
        }
        if !self.plan.audio.is_empty()
            || !self.plan.titles.is_empty()
            || !request.project.effects.is_empty()
        {
            notices.push("Audio, titles and color effects are not previewed yet.");
        }
        let started = Instant::now();
        let image = loop {
            if cancelled() {
                return Err("Preview cancelled.".into());
            }
            if let Some(image) = source.renderer.draw()? {
                break image;
            }
            if started.elapsed() >= Duration::from_secs(30) {
                return Err("The native replay scene could not finish GPU preparation. Seek again to retry.".into());
            }
            wait();
        };
        if self
            .placeholder
            .as_ref()
            .is_none_or(|page| page.dimensions() != size)
        {
            let bytes = recording_studio::video::frame_byte_len(size)?;
            self.placeholder = Some(
                UiTexturePage::owned(size, vec![0; bytes].into())
                    .map_err(|error| format!("Replay preview page was rejected: {error:?}"))?,
            );
        }
        Ok(Some(PreviewOutput {
            page: self
                .placeholder
                .as_ref()
                .expect("preview page admitted")
                .clone(),
            image: Some(image),
            notice: (!notices.is_empty()).then(|| notices.join(" ")),
        }))
    }
}
