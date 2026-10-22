//! Export bounds use project timeline seconds and remain separate from clip source trims.

use recording_studio::{MediaTime, Span};

macro_rules! fields {
    ($($kind:ident => $label:literal, $playhead:literal, $member:ident),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub enum ExportRangeField { $($kind),+ }
        impl ExportRangeField {
            pub const ALL: &[Self] = &[$(Self::$kind),+];
            /// One field registry owns labels, iteration order and the corresponding span endpoint.
            pub const fn label(self) -> &'static str { match self { $(Self::$kind => $label),+ } }
            /// Playhead actions identify the export boundary they replace.
            pub const fn playhead_label(self) -> &'static str { match self { $(Self::$kind => $playhead),+ } }
            /// Displayed values retain the saved project clock without source-clip retiming.
            pub const fn value(self, span: Span) -> MediaTime { match self { $(Self::$kind => span.$member),+ } }
        }
    };
}
fields! { In => "Export In (seconds)", "Set In at playhead", start, Out => "Export Out (seconds)", "Set Out at playhead", end }
