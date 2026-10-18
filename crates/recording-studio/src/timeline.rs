//! Frame-independent timeline gestures produce one reversible metadata transaction on release.

use crate::{Id, MediaTime, Project, Span, StudioError, edit::Command};

mod group;
mod keys;
mod snapping;
pub use group::{
    delete_selection, duplicate_selection, ensure_unlocked, move_selection, selection_ids,
};
pub use keys::{KeyKind, TimelineKey, key_time, move_key};
pub use snapping::SnapIndex;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DragMode {
    Move,
    TrimStart,
    TrimEnd,
}

pub struct TimelineEdit {
    pub commands: Vec<Command>,
    pub span: Span,
}

macro_rules! framing_parameters {
    ($($kind:ident => $label:literal),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub enum FramingParameter { $($kind),+ }
        impl FramingParameter {
            pub const ALL: &[Self] = &[$(Self::$kind),+];
            pub const fn label(self) -> &'static str { match self { $(Self::$kind => $label),+ } }
        }
    };
}
framing_parameters! { Opacity => "Opacity", Rotation => "Rotation", Fit => "Layer mode", Left => "Crop left", Right => "Crop right", Top => "Crop top", Bottom => "Crop bottom" }

impl FramingParameter {
    /// Rotation uses degrees; crop and opacity use normalized fractions; layer mode selects Fit or Fill.
    pub fn value(self, framing: crate::model::Framing) -> f64 {
        match self {
            Self::Opacity => framing.opacity,
            Self::Rotation => framing.rotation_degrees,
            Self::Fit => match framing.fit {
                crate::model::FramingFit::Fit => 0.0,
                crate::model::FramingFit::Fill => 1.0,
            },
            Self::Left => framing.crop[0],
            Self::Top => framing.crop[1],
            Self::Right => 1.0 - framing.crop[2],
            Self::Bottom => 1.0 - framing.crop[3],
        }
    }

    /// Numeric labels retain the parameter's own unit instead of treating image rotation as a percentage.
    pub fn formatted(self, framing: crate::model::Framing) -> String {
        match self {
            Self::Rotation => format!("{:.1}°", self.value(framing)),
            Self::Fit => framing.fit.label().into(),
            Self::Opacity | Self::Left | Self::Right | Self::Top | Self::Bottom => {
                format!("{:.1}%", self.value(framing) * 100.0)
            }
        }
    }

    /// Crop edits preserve a nonempty source rectangle and never change unrelated clip metadata.
    pub fn adjusted(
        self,
        mut framing: crate::model::Framing,
        direction: i8,
    ) -> crate::model::Framing {
        let amount = f64::from(direction.signum()) * 0.025;
        let gap_x = (framing.crop[2] - framing.crop[0]).min(0.01);
        let gap_y = (framing.crop[3] - framing.crop[1]).min(0.01);
        match self {
            Self::Opacity => framing.opacity = (framing.opacity + amount).clamp(0.0, 1.0),
            Self::Rotation => {
                let turn = crate::framing::FULL_TURN_DEGREES;
                framing.rotation_degrees = (framing.rotation_degrees.rem_euclid(turn)
                    + f64::from(direction.signum())
                    + turn * 0.5)
                    .rem_euclid(turn)
                    - turn * 0.5;
            }
            Self::Fit => {
                if direction != 0 {
                    framing.fit = if direction < 0 {
                        crate::model::FramingFit::Fit
                    } else {
                        crate::model::FramingFit::Fill
                    };
                }
            }
            Self::Left => {
                framing.crop[0] = (framing.crop[0] + amount).clamp(0.0, framing.crop[2] - gap_x)
            }
            Self::Top => {
                framing.crop[1] = (framing.crop[1] + amount).clamp(0.0, framing.crop[3] - gap_y)
            }
            Self::Right => {
                framing.crop[2] = (framing.crop[2] - amount).clamp(framing.crop[0] + gap_x, 1.0)
            }
            Self::Bottom => {
                framing.crop[3] = (framing.crop[3] - amount).clamp(framing.crop[1] + gap_y, 1.0)
            }
        }
        framing
    }
}

/// The overview and wheel navigation share the same bounded insertion margin after the last clip.
pub fn overview_end(duration: MediaTime) -> MediaTime {
    MediaTime(
        duration
            .0
            .saturating_add((duration.0 / 20).max(1_000_000))
            .min(crate::time::MAX_MEDIA_MICROS)
            .max(1_000_000),
    )
}

/// Resolves an item's current admitted output range without inferring it from painted coordinates.
pub fn item_span(project: &Project, id: Id) -> Result<Span, StudioError> {
    if let Some(clip) = project.clips.iter().find(|item| item.id == id) {
        return clip.span();
    }
    if let Some(audio) = project.audio.iter().find(|item| item.id == id) {
        return audio.span();
    }
    project
        .shots
        .iter()
        .find(|item| item.id == id)
        .map(|item| item.span)
        .or_else(|| {
            project
                .effects
                .iter()
                .find(|item| item.id == id)
                .map(|item| item.span)
        })
        .or_else(|| {
            project
                .titles
                .iter()
                .find(|item| item.id == id)
                .map(|item| item.span)
        })
        .ok_or(StudioError::Missing {
            kind: "timeline item",
            id: id.0,
        })
}

/// Dragging never edits the source media. Visual trims can extend only into real source handles.
pub fn drag(
    project: &Project,
    id: Id,
    mode: DragMode,
    delta: i128,
    ripple: bool,
) -> Result<TimelineEdit, StudioError> {
    let span = item_span(project, id)?;
    if let Some(original) = project.clips.iter().find(|item| item.id == id) {
        let mut clip = original.clone();
        match mode {
            DragMode::Move => {
                clip.at = shift(span.start, delta)?;
                let span = clip.span()?;
                return Ok(TimelineEdit {
                    commands: vec![Command::MoveClip {
                        id,
                        at: clip.at,
                        track: clip.track,
                    }],
                    span,
                });
            }
            DragMode::TrimStart | DragMode::TrimEnd => {
                let media = project
                    .sources
                    .iter()
                    .find(|source| source.id == clip.source)
                    .ok_or(StudioError::Missing {
                        kind: "source",
                        id: clip.source.0,
                    })?;
                let map = original.time_map()?;
                let anchor = if mode == DragMode::TrimStart {
                    span.start
                } else {
                    span.end
                };
                let wanted = shift(anchor, delta)?;
                let source = map.source_boundary(wanted)?;
                if mode == DragMode::TrimStart {
                    let start = source.0.clamp(0, clip.source_range.end.0 - 1);
                    if !ripple {
                        clip.at = MediaTime(
                            map.timeline_boundary(MediaTime(start))?
                                .0
                                .min(span.end.0 - 1),
                        );
                    }
                    clip.source_range.start = MediaTime(start);
                } else {
                    let maximum = if media.kind == crate::model::MediaKind::Image {
                        crate::time::MAX_MEDIA_MICROS
                    } else {
                        media.duration.0
                    };
                    clip.source_range.end =
                        MediaTime(source.0.clamp(clip.source_range.start.0 + 1, maximum));
                }
                let span = clip.span()?;
                let commands = if mode == DragMode::TrimEnd || ripple {
                    vec![Command::TrimClip {
                        id,
                        source: clip.source_range,
                        ripple,
                    }]
                } else {
                    vec![Command::ReplaceClip { clip }]
                };
                return Ok(TimelineEdit { commands, span });
            }
        }
    }
    if mode != DragMode::Move {
        return Err(StudioError::Invalid(
            "trim handles belong to visual clips; use the item's inspector for keyed tracks",
        ));
    }
    let start = shift(span.start, delta)?;
    let shifted = Span::new(start, start.checked_add(span.duration())?)?;
    let command = if let Some(original) = project.effects.iter().find(|item| item.id == id) {
        let mut effect = original.clone();
        effect.span = shifted;
        Command::ReplaceEffect { effect }
    } else if let Some(original) = project.shots.iter().find(|item| item.id == id) {
        let mut shot = original.clone();
        shot.span = shifted;
        Command::ReplaceShot { shot }
    } else if let Some(original) = project.titles.iter().find(|item| item.id == id) {
        let mut title = original.clone();
        title.span = shifted;
        Command::ReplaceTitle { title }
    } else if let Some(original) = project.audio.iter().find(|item| item.id == id) {
        if original.linked_clip.is_some() {
            return Err(StudioError::Invalid(
                "move linked game audio by moving its video clip",
            ));
        }
        let mut audio = original.clone();
        audio.at = shifted.start;
        Command::ReplaceAudio { audio }
    } else {
        return Err(StudioError::Missing {
            kind: "timeline item",
            id: id.0,
        });
    };
    Ok(TimelineEdit {
        commands: vec![command],
        span: shifted,
    })
}

/// Moving to the beginning clamps at zero; exceeding the project time bound remains an error.
fn shift(time: MediaTime, delta: i128) -> Result<MediaTime, StudioError> {
    let value = (i128::from(time.0) + delta).max(0);
    if value > i128::from(crate::time::MAX_MEDIA_MICROS) {
        return Err(StudioError::TimeOverflow);
    }
    Ok(MediaTime(value as u64))
}
