//! Converts observations into ordinary editable timeline commands, never baked-in footage.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::Highlight;
use crate::{
    Id, MediaTime, Project, Span, StudioError,
    edit::{Command, MAX_EDIT_COMMANDS},
    effect::{AutomationOrigin, Curve, CurveKey, Easing, EffectClip, EffectKind},
    model::{Confidence, MarkerKind},
};

pub(super) const TEMPLATE_REVISION: u32 = 2;
const MAX_HIGHLIGHTS: usize = 100_000;
const MAX_EVENT_EFFECTS: usize = 3;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditStyle {
    #[default]
    Clean,
    Pvp,
    Cinematic,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutoEditOptions {
    pub style: EditStyle,
    pub intensity: f64,
    pub zooms: bool,
    pub camera_shake: bool,
    /// Correlated server-status observations remain explicitly labelled in the project.
    pub include_correlated: bool,
}

impl Default for AutoEditOptions {
    fn default() -> Self {
        Self {
            style: EditStyle::Clean,
            intensity: 0.5,
            zooms: true,
            camera_shake: false,
            include_correlated: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct AutoEditProposal {
    pub commands: Vec<Command>,
    pub highlights: usize,
    pub effects: usize,
    pub correlated: usize,
    /// Events that did not fit this draft; the UI must report this instead of implying full coverage.
    pub omitted: usize,
}

/// Makes a bounded, deterministic draft. Reapplying clears only this source's generated items.
pub fn suggest(
    project: &Project,
    source: Id,
    events: &[Highlight],
    options: &AutoEditOptions,
) -> Result<AutoEditProposal, StudioError> {
    project.validate()?;
    crate::error::limit("highlight observations", events.len(), MAX_HIGHLIGHTS)?;
    if !options.intensity.is_finite() || !(0.0..=1.0).contains(&options.intensity) {
        return Err(StudioError::Invalid(
            "automatic edit intensity must be between zero and one",
        ));
    }
    if !project.sources.iter().any(|media| media.id == source) {
        return Err(StudioError::Missing {
            kind: "source",
            id: source.0,
        });
    }
    let mut strongest: BTreeMap<u64, &Highlight> = BTreeMap::new();
    for event in events {
        if event.confidence == Confidence::Correlated && !options.include_correlated {
            continue;
        }
        let entry = strongest.entry(event.sequence).or_insert(event);
        if score(event.kind) > score(entry.kind) {
            *entry = event;
        }
    }
    let mut next_id = project.next_id()?.0;
    let mut proposal = AutoEditProposal {
        commands: vec![Command::ClearAutomation {
            source: Some(source),
        }],
        highlights: 0,
        effects: 0,
        correlated: 0,
        omitted: 0,
    };
    for clip in project
        .clips
        .iter()
        .filter(|clip| clip.source == source && clip.enabled)
    {
        let map = clip.time_map()?;
        let span = map.span();
        let mut last_emphasis: Option<MediaTime> = None;
        for event in strongest.values() {
            let Some(at) = map.timeline_time(event.at)? else {
                continue;
            };
            if proposal.commands.len() + 1 + MAX_EVENT_EFFECTS > MAX_EDIT_COMMANDS {
                proposal.omitted += 1;
                continue;
            }
            let origin = AutomationOrigin {
                source,
                event_sequence: event.sequence,
                template_revision: TEMPLATE_REVISION,
                clip_id: Some(clip.id),
                source_time: Some(event.at),
            };
            let marker = super::source_marker(take_id(&mut next_id)?, source, clip.id, at, event);
            proposal.commands.push(Command::AddMarker { marker });
            proposal.highlights += 1;
            proposal.correlated += usize::from(event.confidence == Confidence::Correlated);
            if matches!(
                event.kind,
                MarkerKind::Hit | MarkerKind::Combo | MarkerKind::Kill
            ) {
                let emphasis = if event.kind == MarkerKind::Kill {
                    1.0
                } else if event.kind == MarkerKind::Combo {
                    0.75
                } else {
                    0.35
                };
                let spacing = if event.kind == MarkerKind::Hit {
                    650_000
                } else {
                    300_000
                };
                if last_emphasis.is_some_and(|last| at.saturating_sub(last).0 < spacing) {
                    continue;
                }
                last_emphasis = Some(at);
                let recipe = recipe(event, options);
                for (kind, amount) in recipe {
                    if kind == EffectKind::Zoom && !options.zooms {
                        continue;
                    }
                    add_pulse(
                        &mut proposal,
                        &mut next_id,
                        origin,
                        at,
                        span,
                        kind,
                        amount * emphasis * options.intensity,
                    )?;
                }
            }
        }
    }
    Ok(proposal)
}

/// Event identity chooses a repeatable variation; cooldowns leave quiet beats instead of stacking every hit.
fn recipe(event: &Highlight, options: &AutoEditOptions) -> [(EffectKind, f64); MAX_EVENT_EFFECTS] {
    use EffectKind::*;
    let variation = (event.sequence ^ event.sequence.rotate_right(13) ^ u64::from(event.combo)) % 3;
    match options.style {
        EditStyle::Clean => match variation {
            0 => [(Zoom, 5.0), (Contrast, 0.12), (Sharpen, 0.10)],
            1 => [(Contrast, 0.16), (Saturation, 0.08), (Sharpen, 0.12)],
            _ => [(Zoom, 3.5), (Vignette, 0.10), (Contrast, 0.08)],
        },
        EditStyle::Pvp => {
            let mut values = match variation {
                0 => [(Zoom, 7.0), (Contrast, 0.20), (Chromatic, 0.003)],
                1 => [(Zoom, 4.0), (Sharpen, 0.20), (Contrast, 0.16)],
                _ => [(Contrast, 0.18), (Vignette, 0.12), (Saturation, 0.10)],
            };
            if options.camera_shake {
                values[2] = (Shake, 0.35);
            }
            values
        }
        EditStyle::Cinematic => match variation {
            0 => [(Zoom, 2.5), (Bloom, 0.15), (Vignette, 0.12)],
            1 => [(Warmth, 0.10), (Contrast, 0.12), (Bloom, 0.10)],
            _ => [(Zoom, 3.0), (SoftFocus, 0.06), (Vignette, 0.14)],
        },
    }
}

/// Gives stronger observations priority over a hit emitted for the same combo-completing event.
fn score(kind: MarkerKind) -> u8 {
    match kind {
        MarkerKind::Kill => 5,
        MarkerKind::Death => 4,
        MarkerKind::Combo => 3,
        MarkerKind::Hit => 2,
        _ => 1,
    }
}

/// Allocates one identifier inside the draft; committing validates uniqueness across all tracks.
fn take_id(next: &mut u64) -> Result<Id, StudioError> {
    let id = Id(*next);
    *next = next
        .checked_add(1)
        .ok_or(StudioError::IdentifiersExhausted)?;
    Ok(id)
}

/// A short eased envelope returns to neutral without instantaneous camera/effect discontinuities.
#[allow(clippy::too_many_arguments)]
fn add_pulse(
    proposal: &mut AutoEditProposal,
    next: &mut u64,
    origin: AutomationOrigin,
    event: MediaTime,
    clip: Span,
    kind: EffectKind,
    amount: f64,
) -> Result<(), StudioError> {
    let (lead, tail) = match kind {
        EffectKind::Contrast | EffectKind::Flash | EffectKind::Chromatic => (20_000, 220_000),
        EffectKind::Warmth | EffectKind::Bloom | EffectKind::SoftFocus => (180_000, 800_000),
        EffectKind::Shake => (30_000, 280_000),
        _ => (80_000, 520_000),
    };
    let start = MediaTime(event.0.saturating_sub(lead).max(clip.start.0));
    let end = MediaTime(event.0.saturating_add(tail).min(clip.end.0));
    if end.0.saturating_sub(start.0) < 3 || amount <= 0.0 {
        return Ok(());
    }
    let span = Span::new(start, end)?;
    let peak = MediaTime(
        event
            .0
            .saturating_sub(start.0)
            .clamp(1, span.duration().0 - 1),
    );
    let effect = EffectClip {
        id: take_id(next)?,
        span,
        kind,
        enabled: true,
        origin: Some(origin),
        amount: Curve {
            keys: vec![
                CurveKey {
                    at: MediaTime::ZERO,
                    value: 0.0,
                    easing: Easing::Linear,
                },
                CurveKey {
                    at: peak,
                    value: amount,
                    easing: Easing::Smooth,
                },
                CurveKey {
                    at: span.duration(),
                    value: 0.0,
                    easing: Easing::Smooth,
                },
            ],
        },
    };
    proposal.commands.push(Command::AddEffect { effect });
    proposal.effects += 1;
    Ok(())
}
