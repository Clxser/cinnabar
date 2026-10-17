//! Two adjacent admitted target states provide deterministic interpolation and bounded lookahead.

use std::{collections::BTreeMap, path::Path, sync::Arc};

use recording_studio::{
    MediaTime,
    archive::{
        ArchiveError, RecordKind, RecordingCursor, RecordingInfo, RecordingReader, ReplayRecord,
    },
    camera::{CameraTarget, MAX_CAMERA_TARGETS},
};

use super::{TARGET_DIRECTORY, TARGET_TRACK_SCHEMA, TargetFrame, codec};

pub struct TargetRecording {
    cursor: RecordingCursor,
    state: BTreeMap<u64, CameraTarget>,
    current: Option<TargetFrame>,
    next: Option<TargetFrame>,
    sampled: Option<TargetFrame>,
    position: Option<MediaTime>,
}

impl TargetRecording {
    /// Older takes can lack targets; a present invalid directory never silently falls back.
    pub fn open_optional(parent: &Path) -> Result<Option<Self>, ArchiveError> {
        let root = parent.join(TARGET_DIRECTORY);
        match std::fs::symlink_metadata(&root) {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => {
                return Err(ArchiveError::Invalid(
                    "camera target track is not a directory",
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(ArchiveError::Io { path: root, source }),
        }
        let reader = RecordingReader::open(&root)?;
        if reader.info().scene_schema != TARGET_TRACK_SCHEMA {
            return Err(ArchiveError::Invalid(
                "unsupported camera target track schema",
            ));
        }
        Ok(Some(Self {
            cursor: reader.into_cursor(),
            state: BTreeMap::new(),
            current: None,
            next: None,
            sampled: None,
            position: None,
        }))
    }

    /// Scene owners compare take metadata before combining independently recorded tracks.
    pub fn info(&self) -> &RecordingInfo {
        self.cursor.reader().info()
    }

    /// Completion bounds include the final interval after the last target change.
    pub fn duration(&self) -> MediaTime {
        self.cursor.reader().duration()
    }

    /// Seeks rebuild a baseline and exactly one following sample before publishing any interpolated pose.
    pub fn seek(
        &mut self,
        at: MediaTime,
        cancelled: &impl Fn() -> bool,
    ) -> Result<TargetFrame, ArchiveError> {
        let result = self.restore(at, cancelled);
        self.finish(result, cancelled)
    }

    /// Forward playback promotes each crossed sample once, retaining only the two adjacent target sets.
    pub fn advance_to(
        &mut self,
        at: MediaTime,
        cancelled: &impl Fn() -> bool,
    ) -> Result<TargetFrame, ArchiveError> {
        if cancelled() {
            self.invalidate();
            return Err(ArchiveError::Cancelled);
        }
        if self.position.is_none_or(|previous| at < previous) || at > self.duration() {
            return Err(ArchiveError::Invalid(
                "target playback needs a seek before advancing",
            ));
        }
        if self.position == Some(at) {
            return self.sampled.clone().ok_or(ArchiveError::Invalid(
                "camera targets need a successful baseline",
            ));
        }
        let result = self.advance(at, cancelled);
        self.finish(result, cancelled)
    }

    /// A failed sibling source invalidates all target drafts until a complete scene seek succeeds.
    pub fn invalidate(&mut self) {
        self.cursor.invalidate();
        self.state.clear();
        self.current = None;
        self.next = None;
        self.sampled = None;
        self.position = None;
    }

    /// Checkpoint timestamps remain source sample times, independent of the requested display cadence.
    fn restore(
        &mut self,
        at: MediaTime,
        cancelled: &impl Fn() -> bool,
    ) -> Result<TargetFrame, ArchiveError> {
        self.state.clear();
        self.next = None;
        let mut sampled = None;
        self.cursor.seek(at, cancelled, |record| {
            apply(record, &mut self.state)?;
            sampled = Some(record.at);
            Ok(())
        })?;
        self.current = Some(self.frame(sampled.ok_or(ArchiveError::Invalid(
            "camera targets need a successful baseline",
        ))?));
        self.read_next(cancelled)?;
        self.sample(at)
    }

    /// The cursor can run one timestamp ahead while the published transport position stays at `at`.
    fn advance(
        &mut self,
        at: MediaTime,
        cancelled: &impl Fn() -> bool,
    ) -> Result<TargetFrame, ArchiveError> {
        if self.next.as_ref().is_some_and(|next| next.at <= at) {
            self.current = self.next.take();
            let mut sampled = None;
            self.cursor.advance_to(at, cancelled, |record| {
                apply(record, &mut self.state)?;
                sampled = Some(record.at);
                Ok(())
            })?;
            if let Some(sampled) = sampled {
                self.current = Some(self.frame(sampled));
            }
            self.read_next(cancelled)?;
        }
        self.sample(at)
    }

    /// Reads every record sharing the next timestamp, including multipart checkpoint boundaries.
    fn read_next(&mut self, cancelled: &impl Fn() -> bool) -> Result<(), ArchiveError> {
        let Some(at) = self.cursor.next_time(cancelled)? else {
            self.next = None;
            return Ok(());
        };
        self.cursor
            .advance_to(at, cancelled, |record| apply(record, &mut self.state))?;
        self.next = Some(self.frame(at));
        Ok(())
    }

    /// Published samples own a sorted immutable actor array rather than a mutable decoder map.
    fn frame(&self, at: MediaTime) -> TargetFrame {
        TargetFrame {
            at,
            targets: self.state.values().cloned().collect::<Vec<_>>().into(),
        }
    }

    /// Births, removals, runtime reuse and teleports take effect at their recorded boundary.
    fn sample(&self, at: MediaTime) -> Result<TargetFrame, ArchiveError> {
        let current = self.current.as_ref().ok_or(ArchiveError::Invalid(
            "camera targets need a successful baseline",
        ))?;
        Ok(interpolate(current, self.next.as_ref(), at))
    }

    /// Invalid or cancelled drafts cannot seed a later publication after a renderer retains an older frame.
    fn finish(
        &mut self,
        result: Result<TargetFrame, ArchiveError>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<TargetFrame, ArchiveError> {
        let result = if cancelled() {
            Err(ArchiveError::Cancelled)
        } else {
            result
        };
        if let Ok(frame) = &result {
            self.position = Some(frame.at);
            self.sampled = Some(frame.clone());
        } else {
            self.invalidate();
        }
        result
    }
}

/// Lifetime checks prevent smoothing through a despawn, a teleport or a reused runtime ID.
fn interpolate(current: &TargetFrame, next: Option<&TargetFrame>, at: MediaTime) -> TargetFrame {
    let Some(next) = next.filter(|next| current.at < next.at && at > current.at) else {
        return TargetFrame {
            at,
            targets: Arc::clone(&current.targets),
        };
    };
    let amount =
        at.saturating_sub(current.at).0 as f64 / next.at.saturating_sub(current.at).0 as f64;
    let targets = current
        .targets
        .iter()
        .map(|target| {
            let mut sampled = target.clone();
            if let Some(future) = next.target(target.runtime_id)
                && target.same_lifetime(future)
                && !future.teleported
            {
                sampled.pose = target.pose.interpolated(future.pose, amount);
            }
            sampled
        })
        .collect::<Vec<_>>()
        .into();
    TargetFrame { at, targets }
}

/// A target checkpoint replaces its full set; ordinary deltas mutate only named identities.
fn apply(
    record: ReplayRecord<'_>,
    state: &mut BTreeMap<u64, CameraTarget>,
) -> Result<(), ArchiveError> {
    match record.kind {
        RecordKind::CheckpointStart => state.clear(),
        RecordKind::CheckpointEnd => {}
        RecordKind::CheckpointPart | RecordKind::Scene => {
            let delta = codec::read(&mut &record.payload[..])?;
            if record.kind == RecordKind::CheckpointPart && !delta.removed.is_empty() {
                return Err(ArchiveError::Invalid("target checkpoint contains removals"));
            }
            for id in delta.removed {
                state.remove(&id);
            }
            for target in delta.upserts {
                if state.len() == MAX_CAMERA_TARGETS && !state.contains_key(&target.runtime_id) {
                    return Err(ArchiveError::Quota);
                }
                state.insert(target.runtime_id, target);
            }
        }
        _ => return Err(ArchiveError::Invalid("unexpected camera target record")),
    }
    Ok(())
}
