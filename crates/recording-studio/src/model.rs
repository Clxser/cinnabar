//! Portable project metadata. Recorded scene data and imported media are never embedded here.

use std::{collections::BTreeSet, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::{
    FrameRate, MediaTime, PlaybackRate, Span, StudioError,
    camera::CameraShot,
    effect::{AutomationOrigin, EffectClip},
};

mod admission;

pub const PROJECT_VERSION: u32 = 1;
pub const PROJECT_EXTENSION: &str = "cinstudio";
pub const MAX_PROJECT_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_PROJECT_ITEMS: usize = 16_384;
pub const MAX_PROJECT_KEYS: usize = 131_072;
pub const MAX_VIDEO_TRACKS: u16 = 8;

#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Id(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Replay,
    Video,
    Audio,
    Image,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaSource {
    pub id: Id,
    pub name: String,
    pub path: PathBuf,
    pub kind: MediaKind,
    pub duration: MediaTime,
    pub size: [u32; 2],
    /// Source cadence is independent of output FPS, preserving high-rate frames during slow motion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_rate: Option<FrameRate>,
    /// Optional immutable content identity, checked by the storage edge when opening media.
    pub sha256: Option<[u8; 32]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Framing {
    /// Normalized source crop: left, top, right, bottom.
    pub crop: [f64; 4],
    pub opacity: f64,
    /// Clockwise image rotation around the retained crop's centre, independent of the replay camera.
    #[serde(default)]
    pub rotation_degrees: f64,
    #[serde(default)]
    pub fit: FramingFit,
}

macro_rules! framing_fits {
    ($default:ident => $default_label:literal, $($kind:ident => $label:literal),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum FramingFit { #[default] $default, $($kind),+ }
        impl FramingFit {
            pub const ALL: &[Self] = &[Self::$default, $(Self::$kind),+];
            /// One definition owns serialized modes, their control labels and inspector order.
            pub const fn label(self) -> &'static str { match self { Self::$default => $default_label, $(Self::$kind => $label),+ } }
        }
    };
}
framing_fits! { Fit => "Fit", Fill => "Fill" }

impl Default for Framing {
    fn default() -> Self {
        Self {
            crop: [0.0, 0.0, 1.0, 1.0],
            opacity: 1.0,
            rotation_degrees: 0.0,
            fit: FramingFit::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Clip {
    pub id: Id,
    pub source: Id,
    pub source_range: Span,
    pub at: MediaTime,
    pub rate: PlaybackRate,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_ramp: Option<crate::retime::SpeedRamp>,
    pub track: u16,
    pub framing: Framing,
    pub enabled: bool,
}

impl Clip {
    /// Compiles one source clock for visual sampling, event anchors and transition handles.
    pub fn time_map(&self) -> Result<crate::retime::TimeMap, StudioError> {
        crate::retime::TimeMap::new(
            self.source_range,
            self.at,
            self.rate,
            self.speed_ramp.as_ref(),
        )
    }

    /// Computes the half-open output range from the retained source and authored speed map.
    pub fn span(&self) -> Result<Span, StudioError> {
        self.time_map().map(|map| map.span())
    }

    /// Maps an admitted clip timestamp to its retained source range.
    pub fn source_time(&self, at: MediaTime) -> Result<MediaTime, StudioError> {
        self.time_map()?.source_time(at)
    }

    /// Continues motion through a transition using source handles outside the trimmed interval.
    /// Only an actual media boundary holds a frame; trimming alone does not freeze the image.
    pub fn transition_time(
        &self,
        at: MediaTime,
        source_duration: MediaTime,
    ) -> Result<MediaTime, StudioError> {
        self.time_map()?.transition_time(at, source_duration)
    }

    /// Maps retained source evidence to this clip's current position after trimming and retiming.
    pub fn timeline_time(&self, source: MediaTime) -> Result<Option<MediaTime>, StudioError> {
        self.time_map()?.timeline_time(source)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioRole {
    Game,
    Music,
    Voice,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioClip {
    pub id: Id,
    pub source: Id,
    /// Linked game audio follows the referenced visual clip's cuts, trims, moves and rate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linked_clip: Option<Id>,
    pub source_range: Span,
    pub at: MediaTime,
    pub rate: PlaybackRate,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_ramp: Option<crate::retime::SpeedRamp>,
    pub role: AudioRole,
    pub gain: f64,
    pub fade_in: MediaTime,
    pub fade_out: MediaTime,
    pub muted: bool,
}

impl AudioClip {
    /// Linked and independent sound compile the same map used by visual clips.
    pub fn time_map(&self) -> Result<crate::retime::TimeMap, StudioError> {
        crate::retime::TimeMap::new(
            self.source_range,
            self.at,
            self.rate,
            self.speed_ramp.as_ref(),
        )
    }

    /// Audio owns the same rate mapping as visual clips, with independent gain automation.
    pub fn span(&self) -> Result<Span, StudioError> {
        self.time_map().map(|map| map.span())
    }

    /// Evaluates fades without tying gain to wall time or the real-time audio device.
    pub fn gain_at(&self, at: MediaTime) -> f64 {
        let Ok(span) = self.span() else {
            return 0.0;
        };
        self.gain_in_span(at, span)
    }

    /// Per-sample mixers reuse an admitted span instead of recompiling ramp keys for every PCM frame.
    pub fn gain_in_span(&self, at: MediaTime, span: Span) -> f64 {
        if self.muted || !span.contains(at) {
            return 0.0;
        }
        let since = at.saturating_sub(span.start).0 as f64;
        let until = span.end.saturating_sub(at).0 as f64;
        let fade_in = if self.fade_in.0 == 0 {
            1.0
        } else {
            (since / self.fade_in.0 as f64).min(1.0)
        };
        let fade_out = if self.fade_out.0 == 0 {
            1.0
        } else {
            (until / self.fade_out.0 as f64).min(1.0)
        };
        self.gain * fade_in * fade_out
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TitleStyle {
    Body,
    Heading,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TitleClip {
    pub id: Id,
    pub span: Span,
    pub text: String,
    pub anchor: [f64; 2],
    pub size: f64,
    pub color: [f64; 4],
    pub style: TitleStyle,
    pub shadow: bool,
    pub fade_in: MediaTime,
    pub fade_out: MediaTime,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkerKind {
    Manual,
    Beat,
    Hit,
    Combo,
    Death,
    Kill,
    RoundStart,
    RoundEnd,
    CameraCut,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    Observed,
    Correlated,
    Confirmed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Marker {
    pub id: Id,
    pub at: MediaTime,
    pub kind: MarkerKind,
    pub label: String,
    pub confidence: Confidence,
    pub origin: Option<AutomationOrigin>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionKind {
    Crossfade,
    DipToBlack,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transition {
    pub id: Id,
    pub from: Id,
    pub to: Id,
    pub duration: MediaTime,
    pub kind: TransitionKind,
}

impl Transition {
    /// The cut is centred in one shared half-open transition interval, including source handles.
    pub fn span_at(&self, cut: MediaTime) -> Result<Span, StudioError> {
        let start = MediaTime(cut.0.saturating_sub(self.duration.0 / 2));
        Span::new(start, start.checked_add(self.duration)?)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ExportSettings {
    /// Half-open project timeline range; absent preserves whole-project output for older projects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<Span>,
    pub size: [u32; 2],
    pub fps: FrameRate,
    pub codec: ExportCodec,
    pub encoder: Encoder,
    pub quality: u8,
    pub include_audio: bool,
    /// Prepared source HUD/chat are hidden unless explicitly enabled; raster video pixels are unchanged.
    #[serde(default)]
    pub include_recorded_hud: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportCodec {
    #[default]
    H264,
    Hevc,
    Av1,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Encoder {
    #[default]
    Auto,
    Cpu,
    Nvidia,
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            range: None,
            size: [1920, 1080],
            fps: FrameRate::default(),
            codec: ExportCodec::H264,
            encoder: Encoder::Auto,
            quality: 20,
            include_audio: true,
            include_recorded_hud: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub version: u32,
    pub name: String,
    pub sources: Vec<MediaSource>,
    pub clips: Vec<Clip>,
    pub shots: Vec<CameraShot>,
    pub effects: Vec<EffectClip>,
    pub audio: Vec<AudioClip>,
    pub titles: Vec<TitleClip>,
    pub markers: Vec<Marker>,
    pub transitions: Vec<Transition>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub muted_tracks: BTreeSet<crate::tracks::Track>,
    pub export: ExportSettings,
}

impl Project {
    /// Creates an empty editable project; opening the studio does not begin a recording.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            version: PROJECT_VERSION,
            name: name.into(),
            sources: Vec::new(),
            clips: Vec::new(),
            shots: Vec::new(),
            effects: Vec::new(),
            audio: Vec::new(),
            titles: Vec::new(),
            markers: Vec::new(),
            transitions: Vec::new(),
            muted_tracks: BTreeSet::new(),
            export: ExportSettings::default(),
        }
    }

    /// Decodes bounded metadata and applies all project admission rules before use.
    pub fn from_json(bytes: &[u8]) -> Result<Self, StudioError> {
        crate::error::limit("project bytes", bytes.len(), MAX_PROJECT_BYTES)?;
        let project: Self = serde_json::from_slice(bytes)?;
        project.validate()?;
        Ok(project)
    }

    /// Produces a portable project description; the storage edge performs atomic publication.
    pub fn to_json(&self) -> Result<Vec<u8>, StudioError> {
        self.validate()?;
        let bytes = serde_json::to_vec_pretty(self)?;
        crate::error::limit("project bytes", bytes.len(), MAX_PROJECT_BYTES)?;
        Ok(bytes)
    }

    /// A project-local identifier never reuses one still referenced by any track.
    pub fn next_id(&self) -> Result<Id, StudioError> {
        self.ids()
            .map(|id| id.0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .map(Id)
            .ok_or(StudioError::IdentifiersExhausted)
    }

    /// Includes audio, titles and camera-only scenes so exports need not end at the last video.
    pub fn duration(&self) -> MediaTime {
        let visual = self
            .clips
            .iter()
            .filter_map(|clip| clip.span().ok())
            .map(|span| span.end);
        let audio = self
            .audio
            .iter()
            .filter_map(|clip| clip.span().ok())
            .map(|span| span.end);
        visual
            .chain(audio)
            .chain(self.shots.iter().map(|shot| shot.span.end))
            .chain(self.titles.iter().map(|title| title.span.end))
            .chain(self.effects.iter().map(|effect| effect.span.end))
            .max()
            .unwrap_or(MediaTime::ZERO)
    }

    /// Iterates only stable item identities, without visiting media or keyframe payloads.
    pub(crate) fn ids(&self) -> impl Iterator<Item = Id> + '_ {
        self.sources
            .iter()
            .map(|v| v.id)
            .chain(self.clips.iter().map(|v| v.id))
            .chain(self.shots.iter().map(|v| v.id))
            .chain(self.effects.iter().map(|v| v.id))
            .chain(self.audio.iter().map(|v| v.id))
            .chain(self.titles.iter().map(|v| v.id))
            .chain(self.markers.iter().map(|v| v.id))
            .chain(self.transitions.iter().map(|v| v.id))
    }
}
