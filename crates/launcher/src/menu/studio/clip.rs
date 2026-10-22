//! Clip time fields share their labels and bounded input contract across native presentation owners.

use recording_studio::{MediaTime, model::Clip};

pub const MAX_CLIP_TIME_BYTES: usize = 32;

macro_rules! clip_time_fields {
    ($($field:ident => $label:literal => $($member:ident).+),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub enum ClipTimeField { $($field),+ }

        impl ClipTimeField {
            pub const ALL: &[Self] = &[$(Self::$field),+];

            /// Labels identify seconds and distinguish timeline position from source handles.
            pub const fn label(self) -> &'static str {
                match self { $(Self::$field => $label),+ }
            }

            /// Reads the saved microsecond clock without applying output-frame quantization.
            pub fn value(self, clip: &Clip) -> MediaTime {
                match self { $(Self::$field => clip.$($member).+),+ }
            }
        }
    };
}

clip_time_fields! {
    Position => "Position (seconds)" => at,
    SourceIn => "Source In (seconds)" => source_range.start,
    SourceOut => "Source Out (seconds)" => source_range.end,
}
