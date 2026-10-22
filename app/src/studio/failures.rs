//! Native track failures close only their own admission; capture mode decides whether POV may continue.

use std::collections::BTreeMap;

use bevy::prelude::World;
use recording_studio::{
    MediaTime,
    capture::{CaptureMode, CaptureTrack},
};

use super::Active;

type Sample = fn(&World, &mut Active, MediaTime) -> Result<(), String>;

macro_rules! native_tracks {
    ($($kind:ident => ($field:ident, $label:literal, $schema:path, $directory:path, $sample:expr)),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
        pub(super) enum NativeTrack {
            $($kind),+
        }
        impl NativeTrack {
            const ALL: &'static [Self] = &[$(Self::$kind),+];

            /// One registry owns native sampling, worker polling and final metadata.
            fn label(self) -> &'static str {
                match self { $(Self::$kind => $label),+ }
            }

            /// Terrain observes its own publication stream; the other producers share cadence sampling.
            fn sampler(self) -> Option<Sample> {
                match self { $(Self::$kind => $sample),+ }
            }

            /// Polls each concrete native worker without moving or cancelling its owner.
            fn error(self, active: &mut Active) -> Option<String> {
                match self {
                    $(Self::$kind => active.$field.as_mut().and_then(|track| track.capture.error())),+
                }
            }

            /// Ownership moves only on the finalizer, where blocking queue drains are permitted.
            fn finish(self, active: &mut Active, at: MediaTime) -> Result<(), String> {
                match self {
                    $(Self::$kind => active.$field
                        .take()
                        .ok_or_else(|| format!("The {} recording track is unavailable.", self.label()))?
                        .capture
                        .finish(at)),+
                }
            }

            /// Native schemas and directories come from their owners; interruption is never marked complete.
            fn metadata(self, failure: Option<&TrackFailure>) -> CaptureTrack {
                let (schema, directory) = match self { $(Self::$kind => ($schema, $directory)),+ };
                CaptureTrack {
                    schema: schema.into(),
                    directory: directory.into(),
                    complete: failure.is_none(),
                    error: failure.map(|failure| {
                        format!("{} capture failed at {:.3}s: {}", self.label(), failure.at.seconds(), failure.error)
                            .chars().take(512).collect()
                    }),
                }
            }
        }
    };
}

native_tracks! {
    Terrain => (terrain, "terrain", recording_scene::TERRAIN_TRACK_SCHEMA, recording_scene::journal::TERRAIN_DIRECTORY, None),
    Environment => (environment, "environment", recording_scene::environment::ENVIRONMENT_TRACK_SCHEMA, recording_scene::environment::ENVIRONMENT_DIRECTORY, Some(super::environment::sample as Sample)),
    Actors => (actors, "actors", recording_scene::actors::ACTOR_TRACK_SCHEMA, recording_scene::actors::ACTOR_DIRECTORY, Some(super::actors::sample as Sample)),
    Targets => (targets, "subjects", recording_scene::targets::TARGET_TRACK_SCHEMA, recording_scene::targets::TARGET_DIRECTORY, Some(super::targets::sample as Sample)),
    Visuals => (visuals, "visuals", recording_scene::visuals::VISUAL_TRACK_SCHEMA, recording_scene::visuals::VISUAL_DIRECTORY, Some(super::visuals::sample as Sample)),
}

struct TrackFailure {
    at: MediaTime,
    error: String,
}

#[derive(Default)]
pub(super) struct NativeFailures {
    tracks: BTreeMap<NativeTrack, TrackFailure>,
    message: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Admission {
    Continue,
    Stop,
    StopAt(MediaTime),
}

impl NativeFailures {
    /// A failed producer is never sampled or polled again while its owned worker is retained for draining.
    pub(super) fn contains(&self, track: NativeTrack) -> bool {
        self.tracks.contains_key(&track)
    }

    /// Native coverage notices are cached at failure boundaries, avoiding per-frame string allocation.
    pub(super) fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    /// The first failure of each native track retains its original cutoff and cause.
    fn record(&mut self, mode: CaptureMode, track: NativeTrack, at: MediaTime, error: String) {
        if self.contains(track) {
            return;
        }
        self.message = Some(if mode.records_video() {
            format!(
                "Video recording continues; native {} replay is incomplete (capture failed at {:.3}s): {error}",
                track.label(),
                at.seconds()
            )
        } else {
            format!(
                "3D replay stopped after {} capture failed at {:.3}s: {error}. Verifying the common committed prefix while saving.",
                track.label(),
                at.seconds()
            )
        });
        self.tracks.insert(track, TrackFailure { at, error });
    }

    /// A primary/video failure always stops; scene-only failure permits continued capture only with a POV proxy.
    fn admission(&self, mode: CaptureMode, primary_failed: bool) -> Admission {
        if !mode.records_video()
            && let Some(at) = self.tracks.values().map(|failure| failure.at).min()
        {
            Admission::StopAt(at)
        } else if primary_failed {
            Admission::Stop
        } else {
            Admission::Continue
        }
    }
}

/// Applying the common mode policy never reads storage or waits for any capture worker.
fn reconcile(active: &mut Active, now: f64) {
    match active
        .native_failures
        .admission(active.session.settings().mode, active.failure.is_some())
    {
        Admission::Continue => {}
        Admission::Stop => super::stop(active, now),
        Admission::StopAt(at) => {
            if active.failure.is_none() {
                active.failure = Some(
                    active
                        .native_failures
                        .message()
                        .unwrap_or("Native replay capture failed.")
                        .to_owned(),
                );
            }
            super::stop_at(active, now, at);
        }
    }
}

/// Freezes failed native admission at its source time while preserving accepted queued work for finalization.
pub(super) fn interrupt(active: &mut Active, track: NativeTrack, error: String, now: f64) {
    if active.native_failures.contains(track) {
        return;
    }
    let at = MediaTime::from_seconds((now - active.started).max(0.0))
        .unwrap_or(MediaTime(recording_studio::time::MAX_MEDIA_MICROS));
    bevy::log::warn!(native_track = track.label(), error = %error, mode = ?active.session.settings().mode, "Studio native track interrupted");
    active
        .native_failures
        .record(active.session.settings().mode, track, at, error);
    reconcile(active, now);
}

/// Primary metadata, video readback and encoder failures remain fatal independently of native mode policy.
pub(super) fn fatal(active: &mut Active, error: String, now: f64) {
    active.failure.get_or_insert(error);
    reconcile(active, now);
}

/// Worker failures use the same policy as immediate producer admission failures.
pub(super) fn poll(active: &mut Active, now: f64) {
    for &track in NativeTrack::ALL {
        if !active.native_failures.contains(track)
            && let Some(error) = track.error(active)
        {
            interrupt(active, track, error, now);
        }
    }
}

/// Each native producer is independent; a failed optional native track cannot cut off a healthy POV path.
pub(super) fn sample(world: &World, active: &mut Active, at: MediaTime, now: f64) {
    for &track in NativeTrack::ALL {
        if active.stopping.is_some() {
            break;
        }
        if !active.native_failures.contains(track)
            && let Some(sample) = track.sampler()
            && let Err(error) = sample(world, active, at)
        {
            interrupt(active, track, error, now);
        }
    }
}

/// Only the existing finalization worker drains tracks; failed owners finish at their own cutoff, not POV end.
pub(super) fn finish(active: &mut Active, duration: MediaTime) -> Option<String> {
    let mode = active.session.settings().mode;
    let mut metadata_error = None;
    for &track in NativeTrack::ALL {
        let at = active
            .native_failures
            .tracks
            .get(&track)
            .map_or(duration, |failure| failure.at.min(duration));
        if let Err(error) = track.finish(active, at) {
            active.native_failures.record(mode, track, at, error);
        }
        let metadata = track.metadata(active.native_failures.tracks.get(&track));
        if let Err(error) = active.session.scene_track(metadata) {
            metadata_error.get_or_insert(error);
        }
    }
    metadata_error
}

#[cfg(test)]
mod tests;
