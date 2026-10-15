//! Converts observations into ordinary editable timeline commands, never baked-in footage.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::Highlight;
use crate::{
    Id, MediaTime, Project, Span, StudioError,
    edit::{Command, MAX_EDIT_COMMANDS},
    effect::{AutomationOrigin, Curve, CurveKey, Easing, EffectClip, EffectKind},
    model::{Confidence, Marker, MarkerKind},
};

const TEMPLATE_REVISION: u32 = 1;
const MAX_HIGHLIGHTS: usize = 100_000;

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
        for event in strongest.values() {
            let Some(at) = clip.timeline_time(event.at)? else {
                continue;
            };
            // Each observed event can introduce a marker and up to three effects.
            if proposal.commands.len() + 4 > MAX_EDIT_COMMANDS {
                proposal.omitted += 1;
                continue;
            }
            let origin = AutomationOrigin {
                source,
                event_sequence: event.sequence,
                template_revision: TEMPLATE_REVISION,
            };
            let marker = Marker {
                id: take_id(&mut next_id)?,
                at,
                kind: event.kind,
                label: event.label.clone(),
                confidence: event.confidence,
                origin: Some(origin),
            };
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
                if options.zooms {
                    add_pulse(
                        &mut proposal,
                        &mut next_id,
                        origin,
                        at,
                        clip.span()?,
                        EffectKind::Zoom,
                        emphasis * options.intensity * 10.0,
                    )?;
                }
                if options.style == EditStyle::Pvp && options.camera_shake {
                    add_pulse(
                        &mut proposal,
                        &mut next_id,
                        origin,
                        at,
                        clip.span()?,
                        EffectKind::Shake,
                        emphasis * options.intensity * 0.6,
                    )?;
                }
                if options.style == EditStyle::Cinematic && event.kind == MarkerKind::Kill {
                    add_pulse(
                        &mut proposal,
                        &mut next_id,
                        origin,
                        at,
                        clip.span()?,
                        EffectKind::Vignette,
                        options.intensity * 0.2,
                    )?;
                }
            }
        }
    }
    Ok(proposal)
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
    let start = MediaTime(event.0.saturating_sub(80_000).max(clip.start.0));
    let end = MediaTime(event.0.saturating_add(420_000).min(clip.end.0));
    if end.0.saturating_sub(start.0) < 3 || amount <= 0.0 {
        return Ok(());
    }
    let span = Span::new(start, end)?;
    let peak = MediaTime((span.duration().0 / 3).max(1));
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
