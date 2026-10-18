//! Native coverage describes saved evidence, independent of the current live connection.

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum WorldLabelCoverage {
    #[default]
    Unavailable,
    /// Exact prepared artwork includes eligible received actors behind the source camera, within
    /// recorded viewer range, server visibility/settings and the bounded native preparation budget.
    RecordedViewer,
}

impl WorldLabelCoverage {
    /// Compact inspector status names the actual scope without claiming complete world reception.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Unavailable => "World labels unavailable",
            Self::RecordedViewer => "World labels: recorded viewer policy",
        }
    }

    /// Missing/off-range artwork cannot be fabricated from subject names or the live world.
    pub const fn notice(self, custom_camera: bool) -> Option<&'static str> {
        match self {
            Self::Unavailable => Some(
                "This take has no saved world-label artwork. Record a new take with nametags enabled; subject names cannot reconstruct it.",
            ),
            Self::RecordedViewer if custom_camera => Some(
                "World labels retain recorded viewer range, server visibility/settings and preparation limits. For a missing label, use the Recorded camera or record a new take near the subject with names enabled.",
            ),
            Self::RecordedViewer => None,
        }
    }
}
