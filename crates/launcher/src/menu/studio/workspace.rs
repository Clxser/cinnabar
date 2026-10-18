//! Editor panels and density are view preferences, independent of project edits and undo.

/// Defines each section, its name and its iteration order together.
macro_rules! sections {
    ($name:ident { $default:ident => $first:literal $(, $variant:ident => $label:literal)* $(,)? }) => {
        #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
        pub enum $name { #[default] $default, $($variant),* }
        impl $name {
            pub const ALL: &[Self] = &[Self::$default, $(Self::$variant),*];
            /// Short names identify both desktop tabs and compact selectors.
            pub const fn label(self) -> &'static str {
                match self { Self::$default => $first, $(Self::$variant => $label),* }
            }
        }
    };
}

sections! { Toolbox { Media => "Media", Cameras => "Cameras", Effects => "Effects", Text => "Text", Audio => "Audio" } }
sections! { InspectorSection { Clip => "Clip", Framing => "Frame", Speed => "Speed", Cut => "Cut" } }
sections! { ViewportTool { Navigate => "Navigate", Players => "Players", Path => "Path" } }
sections! { SettingsPage { Recording => "Recording", Preview => "Preview", Export => "Export", Diagnostics => "Setup", Controls => "Controls" } }

/// Workspace controls change only presentation; they never dirty a project.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspaceAction {
    Toolbox(Toolbox),
    Inspector(InspectorSection),
    ToggleInspector,
    PreviewOnly,
    Density,
    TimelineSize,
    ViewportTool(ViewportTool),
    TogglePreviewFill,
    CyclePreviewQuality,
    SettingsPage(SettingsPage),
    DismissStatus,
}

/// Studio scales its controls independently while keeping text and hit geometry in agreement.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Density {
    #[default]
    Compact,
    Comfortable,
}

impl Density {
    /// Compact preserves the familiar OreUI controls while fitting more editor content on screen.
    pub const fn scale(self) -> f32 {
        match self {
            Self::Compact => 0.82,
            Self::Comfortable => 1.0,
        }
    }

    /// Names the active presentation density on its toggle.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Compact => "Compact",
            Self::Comfortable => "Comfortable",
        }
    }

    /// Cycles the supported densities without changing project content.
    pub const fn next(self) -> Self {
        match self {
            Self::Compact => Self::Comfortable,
            Self::Comfortable => Self::Compact,
        }
    }
}

/// Explicit sizes keep the timeline usable on both short laptops and larger editing displays.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TimelineSize {
    #[default]
    Small,
    Medium,
    Large,
}

impl TimelineSize {
    /// Reserves a share of the editor height after fixed toolbars.
    pub const fn fraction(self) -> f32 {
        match self {
            Self::Small => 0.28,
            Self::Medium => 0.48,
            Self::Large => 0.62,
        }
    }

    /// Names the active timeline allocation on its toggle.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Small => "Timeline S",
            Self::Medium => "Timeline M",
            Self::Large => "Timeline L",
        }
    }

    /// Cycles track and preview space without changing timeline zoom.
    pub const fn next(self) -> Self {
        match self {
            Self::Small => Self::Medium,
            Self::Medium => Self::Large,
            Self::Large => Self::Small,
        }
    }
}
