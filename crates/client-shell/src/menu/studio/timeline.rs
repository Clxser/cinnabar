//! Timeline gestures keep project changes transactional and navigation independent of undo history.

use super::*;
use launcher::menu::studio::EditorPanel;
use recording_studio::timeline::{DragMode, SnapIndex, TimelineKey};
use std::collections::BTreeSet;
mod gestures;
mod selection;
#[cfg(test)]
mod tests;

pub(super) enum Gesture {
    Overview {
        left: i32,
        width: u32,
    },
    Edit {
        id: Id,
        mode: DragMode,
        width: u32,
        down: f32,
        duration: MediaTime,
        project: Arc<Project>,
        revision: u64,
        selected: BTreeSet<Id>,
        edges: Vec<MediaTime>,
        snap: SnapIndex,
        last: Option<([f32; 2], bool)>,
        pending: Option<Vec<Command>>,
    },
    Key {
        key: TimelineKey,
        width: u32,
        height: u32,
        down: [f32; 2],
        duration: MediaTime,
        project: Arc<Project>,
        revision: u64,
        snap: SnapIndex,
        last: Option<([f32; 2], bool)>,
        pending: Option<(Command, TimelineKey)>,
    },
}

impl Studio {
    /// Navigation never exposes a time window beyond the project plus a small insertion margin.
    pub(in crate::menu::studio) fn timeline_end(&self) -> u64 {
        let duration = self
            .view
            .project
            .as_ref()
            .map_or(MediaTime::ZERO, |project| project.duration());
        recording_studio::timeline::overview_end(duration).0
    }

    /// Horizontal wheel units retain trackpad precision instead of rounding to button-sized jumps.
    pub(in crate::menu::studio) fn pan_timeline(&mut self, fraction: f64) {
        if !fraction.is_finite() {
            return;
        }
        let maximum = self
            .timeline_end()
            .saturating_sub(self.view.visible_duration.0);
        let start =
            self.view.visible_start.0 as f64 + fraction * self.view.visible_duration.0 as f64;
        Arc::make_mut(&mut self.view).visible_start =
            MediaTime(start.clamp(0.0, maximum as f64).round() as u64);
    }

    /// Zoom retains the time under the pointer, even at fractional wheel steps and project edges.
    pub(in crate::menu::studio) fn zoom_timeline(&mut self, steps: f64, anchor: f64) {
        if !steps.is_finite() || !anchor.is_finite() {
            return;
        }
        let anchor = anchor.clamp(0.0, 1.0);
        let old = self.view.visible_duration.0;
        let minimum = self
            .view
            .project
            .as_ref()
            .and_then(|project| project.export.fps.frame_time(8).ok())
            .map_or(100_000, |time| time.0.max(1_000));
        let end = self.timeline_end();
        let duration = ((old as f64 * 2.0_f64.powf(-steps.clamp(-12.0, 12.0))) as u64)
            .clamp(minimum.min(end), end);
        let at = self.view.visible_start.0 as f64 + old as f64 * anchor;
        let start = (at - duration as f64 * anchor).clamp(0.0, end.saturating_sub(duration) as f64);
        let view = Arc::make_mut(&mut self.view);
        view.visible_duration = MediaTime(duration);
        view.visible_start = MediaTime(start.round() as u64);
    }

    /// Fits the whole admitted project without changing the edit or playback position.
    pub(super) fn fit_timeline(&mut self) {
        let end = self.timeline_end();
        let view = Arc::make_mut(&mut self.view);
        view.visible_start = MediaTime::ZERO;
        view.visible_duration = MediaTime(end);
    }

    /// Explicit reveal keeps manual panning useful while making a lost playhead easy to find.
    pub(super) fn reveal_playhead(&mut self) {
        let maximum = self
            .timeline_end()
            .saturating_sub(self.view.visible_duration.0);
        let start = self
            .transport
            .position()
            .0
            .saturating_sub(self.view.visible_duration.0 / 3)
            .min(maximum);
        Arc::make_mut(&mut self.view).visible_start = MediaTime(start);
    }

    /// Playback follows the playhead and republishes graph samples for the same visible window.
    pub(super) fn follow_playhead(&mut self) {
        if self.transport.playing()
            && self.view.workspace.follow_playhead
            && (self.transport.position() < self.view.visible_start
                || self.transport.position().0
                    >= self
                        .view
                        .visible_start
                        .0
                        .saturating_add(self.view.visible_duration.0 * 9 / 10))
        {
            self.reveal_playhead();
        }
        self.reconcile_graph_plot();
    }

    /// The caller supplies the last painted scroll viewport in window-logical pixels.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::menu) fn timeline_wheel(
        &mut self,
        point: ui::UiPoint,
        viewport: Option<ui::UiRect>,
        ruler: bool,
        horizontal: f32,
        vertical: f32,
        pixels: bool,
        shift: bool,
        zoom: bool,
    ) -> bool {
        if self.view.show_library || self.view.workflow.show_settings || self.view.project.is_none()
        {
            return false;
        }
        let Some(viewport) = viewport else {
            return false;
        };
        if !viewport.contains(point) && !ruler {
            return false;
        }
        if !horizontal.is_finite() || !vertical.is_finite() {
            return false;
        }
        let width = (viewport.max().x() - viewport.min().x()).max(1.0);
        if zoom {
            let steps = f64::from(vertical) / if pixels { 160.0 } else { 4.0 };
            self.zoom_timeline(steps, f64::from((point.x() - viewport.min().x()) / width));
            return true;
        }
        if horizontal != 0.0 || shift || ruler {
            let amount = if horizontal != 0.0 {
                horizontal
            } else {
                vertical
            };
            let fraction = -f64::from(amount) / if pixels { f64::from(width) } else { 12.0 };
            self.pan_timeline(fraction);
            return true;
        }
        false
    }
}
