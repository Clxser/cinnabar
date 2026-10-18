//! Structural admission shared by imports, edits, autosave and export.

use std::collections::{BTreeMap, BTreeSet};

use super::*;
use crate::{camera::CameraMode, error::limit, time::MAX_MEDIA_MICROS};

impl Project {
    /// Rejects malformed cross-references and bounds work before any preview or encoder consumes it.
    pub fn validate(&self) -> Result<(), StudioError> {
        if self.version != PROJECT_VERSION {
            return Err(StudioError::Version(self.version));
        }
        limit("project name bytes", self.name.len(), 256)?;
        if self.muted_tracks.iter().any(|track| !track.valid()) {
            return Err(StudioError::Invalid("muted track is outside the timeline"));
        }
        let mut ids = BTreeSet::new();
        for id in self.ids() {
            if id.0 == 0 || !ids.insert(id) {
                return Err(StudioError::Invalid(
                    "project identifiers must be unique and nonzero",
                ));
            }
        }
        limit("project items", ids.len(), MAX_PROJECT_ITEMS)?;
        let mut keys = 0usize;
        for shot in &self.shots {
            shot.validate()?;
            keys += shot.fov_curve.as_ref().map_or(0, |curve| curve.keys.len());
            if let CameraMode::Path { keys: path, .. } = &shot.mode {
                keys += path.len();
            }
        }
        for effect in &self.effects {
            effect.span.validate()?;
            effect.amount.validate(effect.span.duration())?;
            keys += effect.amount.keys.len();
        }
        let sources: BTreeMap<_, _> = self
            .sources
            .iter()
            .map(|source| (source.id, source))
            .collect();
        let clips: BTreeMap<_, _> = self.clips.iter().map(|clip| (clip.id, clip)).collect();
        for source in &self.sources {
            if let Some(fps) = source.frame_rate {
                fps.validate()?;
            }
            limit("source name bytes", source.name.len(), 256)?;
            if source.path.as_os_str().is_empty()
                || source.path.as_os_str().len() > 32_768
                || source.size.iter().any(|&side| side > 32_768)
                || source.kind != MediaKind::Audio && source.size.contains(&0)
                || source.duration.0 > MAX_MEDIA_MICROS
                || source.kind != MediaKind::Image && source.duration == MediaTime::ZERO
            {
                return Err(StudioError::Invalid(
                    "media sources need a bounded path, dimensions and valid duration",
                ));
            }
        }
        for origin in self
            .effects
            .iter()
            .filter_map(|effect| effect.origin)
            .chain(self.markers.iter().filter_map(|marker| marker.origin))
        {
            source(&sources, origin.source)?;
            match (origin.clip_id, origin.source_time) {
                (None, None) => {}
                (Some(id), Some(at)) => {
                    let clip = clips.get(&id).ok_or(StudioError::Missing {
                        kind: "generated item clip",
                        id: id.0,
                    })?;
                    if clip.source != origin.source || !clip.source_range.contains(at) {
                        return Err(StudioError::Invalid(
                            "generated item anchor must belong to its retained source clip",
                        ));
                    }
                }
                _ => {
                    return Err(StudioError::Invalid(
                        "generated items need both clip and source-time anchors",
                    ));
                }
            }
        }
        let mut lanes: BTreeMap<u16, Vec<Span>> = BTreeMap::new();
        for clip in &self.clips {
            let source = source(&sources, clip.source)?;
            clip.source_range.validate()?;
            clip.rate.validate()?;
            if let Some(ramp) = &clip.speed_ramp {
                ramp.validate(clip.rate)?;
                keys += ramp.keys.len();
                if source.kind != MediaKind::Image
                    && ramp.keys.iter().any(|key| key.at > source.duration)
                {
                    return Err(StudioError::Invalid(
                        "speed ramp keys extend beyond their source media",
                    ));
                }
            }
            if source.kind == MediaKind::Audio || clip.track >= MAX_VIDEO_TRACKS {
                return Err(StudioError::Invalid(
                    "a visual clip needs a visual source and valid track",
                ));
            }
            if source.kind != MediaKind::Image && clip.source_range.end > source.duration {
                return Err(StudioError::Invalid("clip extends beyond its source media"));
            }
            clip.framing.validate()?;
            let span = clip.span()?;
            if clip.enabled {
                lanes.entry(clip.track).or_default().push(span);
            }
        }
        for spans in lanes.values_mut() {
            spans.sort_by_key(|span| span.start);
            if spans.windows(2).any(|pair| pair[0].overlaps(pair[1])) {
                return Err(StudioError::Invalid(
                    "clips overlap on the same video track",
                ));
            }
        }
        for clip in &self.audio {
            let source = source(&sources, clip.source)?;
            clip.source_range.validate()?;
            clip.rate.validate()?;
            if let Some(ramp) = &clip.speed_ramp {
                ramp.validate(clip.rate)?;
                keys += ramp.keys.len();
                if ramp.keys.iter().any(|key| key.at > source.duration) {
                    return Err(StudioError::Invalid(
                        "audio speed ramp keys extend beyond their source media",
                    ));
                }
            }
            let duration = clip.span()?.duration();
            if let Some(linked) = clip.linked_clip {
                let visual = clips.get(&linked).ok_or(StudioError::Missing {
                    kind: "linked visual clip",
                    id: linked.0,
                })?;
                if clip.at != visual.at
                    || clip.source_range != visual.source_range
                    || clip.rate != visual.rate
                    || clip.speed_ramp != visual.speed_ramp
                {
                    return Err(StudioError::Invalid(
                        "linked audio must retain its visual clip's source interval, position and rate",
                    ));
                }
            }
            if source.kind == MediaKind::Image
                || clip.source_range.end > source.duration
                || !clip.gain.is_finite()
                || !(0.0..=crate::audio::MAX_GAIN).contains(&clip.gain)
                || clip.fade_in > duration
                || clip.fade_out > duration
            {
                return Err(StudioError::Invalid(
                    "audio range, gain or fades are invalid",
                ));
            }
        }
        limit("project curve keys", keys, MAX_PROJECT_KEYS)?;
        for title in &self.titles {
            title.span.validate()?;
            limit(
                "title bytes",
                title.text.len(),
                crate::title::MAX_TITLE_TEXT_BYTES,
            )?;
            if !title
                .anchor
                .iter()
                .chain(&title.color)
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
                || !title.size.is_finite()
                || !(0.01..=1.0).contains(&title.size)
                || title.fade_in > title.span.duration()
                || title.fade_out > title.span.duration()
            {
                return Err(StudioError::Invalid(
                    "title position, color, size or fades are invalid",
                ));
            }
        }
        for marker in &self.markers {
            limit("marker label bytes", marker.label.len(), 512)?;
            if marker.at.0 > MAX_MEDIA_MICROS {
                return Err(StudioError::TimeOverflow);
            }
        }
        let mut cuts = BTreeSet::new();
        for transition in &self.transitions {
            let a = clips.get(&transition.from).ok_or(StudioError::Missing {
                kind: "clip",
                id: transition.from.0,
            })?;
            let b = clips.get(&transition.to).ok_or(StudioError::Missing {
                kind: "clip",
                id: transition.to.0,
            })?;
            if !cuts.insert((transition.from, transition.to))
                || a.track != b.track
                || a.span()?.end != b.at
                || transition.duration == MediaTime::ZERO
                || transition.duration > a.span()?.duration()
                || transition.duration > b.span()?.duration()
            {
                return Err(StudioError::Invalid(
                    "transitions require adjacent clips and a duration within both clips",
                ));
            }
        }
        self.export.fps.validate()?;
        if let Some(range) = self.export.range {
            range.validate()?;
        }
        let [width, height] = self.export.size;
        if width < 16
            || height < 16
            || width > 7680
            || height > 7680
            || width % 2 != 0
            || height % 2 != 0
            || self.export.quality > 51
        {
            return Err(StudioError::Invalid(
                "export dimensions must be even and within 16..=7680 pixels",
            ));
        }
        Ok(())
    }
}

/// Looks up a media source once during admission, preserving a useful missing-source error.
fn source<'a>(
    sources: &BTreeMap<Id, &'a MediaSource>,
    id: Id,
) -> Result<&'a MediaSource, StudioError> {
    sources.get(&id).copied().ok_or(StudioError::Missing {
        kind: "source",
        id: id.0,
    })
}
