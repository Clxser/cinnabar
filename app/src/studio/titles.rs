//! Worker-owned editorial title layout and shared preview/export composition.

use std::{collections::VecDeque, sync::Arc};

use assets::RuntimeFontCatalog;
use recording_studio::{
    playback::{CompiledTimeline, FramePlan},
    preview::PreviewRequest,
};
use render::replay::{FrameTitles, ReplayRenderContext, TitleLayer};
use render_model::text_bitmap::{MAX_TEXT_CACHE_BYTES, MAX_TEXT_LAYERS, TextBitmap};
use render_scene::ui_preview::PreviewImage;

mod bitmap;

struct Cached {
    key: bitmap::Key,
    bitmap: Arc<TextBitmap>,
}

pub(super) struct Titles {
    font: Option<Arc<RuntimeFontCatalog>>,
    layouts: ui::TextLayoutCache,
    cache: VecDeque<Cached>,
    bytes: usize,
    timeline: Option<CompiledTimeline>,
    plan: FramePlan,
    renderer: Option<FrameTitles>,
}

impl Titles {
    /// Font pages and GPU handles are retained immutably; all expensive work occurs on the owning worker.
    pub fn new(context: Option<ReplayRenderContext>) -> Self {
        Self {
            font: None,
            layouts: ui::TextLayoutCache::new(32, 2 * 1024 * 1024),
            cache: VecDeque::new(),
            bytes: 0,
            timeline: None,
            plan: FramePlan::default(),
            renderer: context.map(FrameTitles::new),
        }
    }

    /// Font changes invalidate ink by source identity, including session glyph sheets and face alternatives.
    pub fn set_font(&mut self, font: Arc<RuntimeFontCatalog>) {
        if self
            .font
            .as_ref()
            .is_none_or(|old| !Arc::ptr_eq(old, &font))
        {
            self.cache.clear();
            self.bytes = 0;
            self.layouts = ui::TextLayoutCache::new(32, 2 * 1024 * 1024);
            self.font = Some(font);
        }
    }

    /// Reuses the timeline index across frames and avoids any graphics work when no title is active.
    pub fn active(&mut self, request: &PreviewRequest) -> Result<bool, String> {
        if self
            .timeline
            .as_ref()
            .is_none_or(|timeline| !Arc::ptr_eq(timeline.project(), &request.project))
        {
            self.timeline =
                Some(CompiledTimeline::new(request.project.clone()).map_err(|e| e.to_string())?);
        }
        self.timeline
            .as_ref()
            .expect("title timeline admitted")
            .sample(request.frame_time()?, &mut self.plan, |_, _, _| None)
            .map_err(|e| e.to_string())?;
        Ok(!self.plan.titles.is_empty())
    }

    /// A title keeps screen placement while camera and scene effects change behind it.
    pub fn render(
        &mut self,
        request: &PreviewRequest,
        image: Arc<PreviewImage>,
        cancelled: &impl Fn() -> bool,
        wait: &impl Fn(),
    ) -> Result<Arc<PreviewImage>, String> {
        if !self.active(request)? {
            return Ok(image);
        }
        if self.plan.titles.len() > MAX_TEXT_LAYERS {
            return Err(
                "Too many overlapping titles. Shorten the overlapping title ranges.".into(),
            );
        }
        let font = self
            .font
            .as_ref()
            .ok_or("The title font snapshot is unavailable.")?;
        let size = image.target.size();
        let at = request.frame_time()?;
        let mut layers = Vec::with_capacity(self.plan.titles.len());
        let mut frame_bytes = 0;
        for &index in &self.plan.titles {
            if cancelled() {
                return Err("Title preparation cancelled.".into());
            }
            let title = &request.project.titles[index];
            let opacity = title.opacity_at(at) as f32;
            if opacity <= 0.0 || title.text.is_empty() {
                continue;
            }
            let key = bitmap::Key {
                text: title.text.clone(),
                width: size[0],
                pixels: (title.size * f64::from(size[1])).round().max(1.0) as u32,
                style: title.style,
                rgb: [0, 1, 2].map(|c| (title.color[c] * 255.0).round() as u8),
                shadow: title.shadow,
            };
            let bitmap = if let Some(index) = self.cache.iter().position(|entry| entry.key == key) {
                let cached = self.cache.remove(index).expect("cached title index");
                let bitmap = cached.bitmap.clone();
                self.cache.push_back(cached);
                bitmap
            } else {
                let bitmap = bitmap::raster(font, &mut self.layouts, &key, cancelled)?;
                let cost = bitmap.pixels().len() + key.text.len();
                while self.bytes + cost > MAX_TEXT_CACHE_BYTES
                    || self.cache.len() >= MAX_TEXT_LAYERS
                {
                    let cached = self
                        .cache
                        .pop_front()
                        .ok_or("Title ink exceeds the cache budget.")?;
                    self.bytes -= cached.bitmap.pixels().len() + cached.key.text.len();
                }
                self.bytes += cost;
                self.cache.push_back(Cached {
                    key,
                    bitmap: bitmap.clone(),
                });
                bitmap
            };
            frame_bytes += bitmap.pixels().len();
            if frame_bytes > MAX_TEXT_CACHE_BYTES {
                return Err(
                    "Too much simultaneous title ink. Reduce title sizes or overlap.".into(),
                );
            }
            layers.push(TitleLayer {
                bitmap,
                anchor: title.anchor.map(|v| v as f32),
                opacity,
            });
        }
        if layers.is_empty() {
            return Ok(image);
        }
        let renderer = self
            .renderer
            .as_mut()
            .ok_or("Titles need an initialized graphics device.")?;
        let started = std::time::Instant::now();
        loop {
            if cancelled() {
                return Err("Title composition cancelled.".into());
            }
            if let Some(output) = renderer.render(image.clone(), &layers)? {
                return Ok(output);
            }
            if started.elapsed() >= std::time::Duration::from_secs(30) {
                return Err("Title composition could not acquire an output image.".into());
            }
            wait();
        }
    }
}
