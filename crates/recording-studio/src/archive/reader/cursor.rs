//! One verified segment and one lookahead header advance without replaying previous frames.

use super::{ArchiveError, MediaTime, RecordingReader, ReplayRecord, VerifiedSegment};

pub struct RecordingCursor {
    reader: RecordingReader,
    next_segment: usize,
    segment: Option<VerifiedSegment>,
    payload: Vec<u8>,
    position: Option<MediaTime>,
}

impl RecordingCursor {
    /// Takes ownership of a validated immutable index; no scene or segment is decoded yet.
    pub(super) fn new(reader: RecordingReader) -> Self {
        Self {
            reader,
            next_segment: 0,
            segment: None,
            payload: Vec::new(),
            position: None,
        }
    }

    /// Index metadata remains accessible without reopening a recording during playback.
    pub fn reader(&self) -> &RecordingReader {
        &self.reader
    }

    /// A failed or cancelled operation clears the position and requires a fresh seek.
    pub const fn position(&self) -> Option<MediaTime> {
        self.position
    }

    /// A scene owner's failed object admission invalidates its cursor too, even after journal IO succeeded.
    pub fn invalidate(&mut self) {
        self.position = None;
        self.segment = None;
    }

    /// Reconstructs from the nearest complete checkpoint. Callers discard their draft on error.
    pub fn seek(
        &mut self,
        at: MediaTime,
        cancelled: impl Fn() -> bool,
        mut visit: impl FnMut(ReplayRecord<'_>) -> Result<(), ArchiveError>,
    ) -> Result<(), ArchiveError> {
        let checkpoint = self.reader.checkpoint_index(at)?;
        self.segment = None;
        self.next_segment = checkpoint;
        self.position = None;
        self.run_to(at, &cancelled, &mut visit)
    }

    /// Emits only newly crossed records, including every record sharing a timestamp. It never rewinds.
    /// A corrupt later segment or failing visitor invalidates this cursor and the caller's staging state.
    pub fn advance_to(
        &mut self,
        at: MediaTime,
        cancelled: impl Fn() -> bool,
        mut visit: impl FnMut(ReplayRecord<'_>) -> Result<(), ArchiveError>,
    ) -> Result<(), ArchiveError> {
        let previous = self.position.ok_or(ArchiveError::Invalid(
            "replay cursor needs a successful seek",
        ))?;
        if at < previous || at > self.reader.duration() {
            return Err(ArchiveError::Invalid(
                "forward replay position is outside its admitted interval",
            ));
        }
        self.run_to(at, &cancelled, &mut visit)
    }

    /// Progress is published only after every crossed segment and visitor has succeeded.
    fn run_to(
        &mut self,
        at: MediaTime,
        cancelled: &impl Fn() -> bool,
        visit: &mut impl FnMut(ReplayRecord<'_>) -> Result<(), ArchiveError>,
    ) -> Result<(), ArchiveError> {
        let result = self.visit_new(at, cancelled, visit).and_then(|()| {
            if cancelled() {
                Err(ArchiveError::Cancelled)
            } else {
                Ok(())
            }
        });
        if result.is_ok() {
            self.position = Some(at);
        } else {
            self.position = None;
            self.segment = None;
        }
        result
    }

    /// Segment decompression and checksum work happens once, ahead of the records it exposes.
    fn visit_new(
        &mut self,
        at: MediaTime,
        cancelled: &impl Fn() -> bool,
        visit: &mut impl FnMut(ReplayRecord<'_>) -> Result<(), ArchiveError>,
    ) -> Result<(), ArchiveError> {
        loop {
            if cancelled() {
                return Err(ArchiveError::Cancelled);
            }
            if let Some(segment) = &mut self.segment {
                if !segment.visit_to(at, cancelled, &mut self.payload, visit)? {
                    return Ok(());
                }
                self.segment = None;
            }
            let Some(segment) = self.reader.segments.get(self.next_segment) else {
                return Ok(());
            };
            if segment.start > at {
                return Ok(());
            }
            self.segment = Some(VerifiedSegment::load(
                &self.reader.root,
                *segment,
                cancelled,
            )?);
            self.next_segment += 1;
        }
    }
}
