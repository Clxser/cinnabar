//! Streaming rhythm estimates and exact, editable tempo grids; analysis never retains decoded PCM.

use super::{CHANNELS, ClipDecoder, MAX_BLOCK_FRAMES, sample_frame, sample_time};
use crate::{
    Id, MediaTime, Span, StudioError,
    export::Cancellation,
    model::{AudioClip, Confidence, Marker, MarkerKind},
};
use std::path::Path;

const ANALYSIS_RATE: u32 = 12_000;
const WINDOW_FRAMES: u32 = ANALYSIS_RATE / 100;
pub const MIN_BPM: u16 = 40;
pub const MAX_BPM: u16 = 240;
pub const TEMPO_SCALE: u16 = 10;
pub const MIN_TEMPO: u16 = MIN_BPM * TEMPO_SCALE;
pub const MAX_TEMPO: u16 = MAX_BPM * TEMPO_SCALE;
const TEMPOS: usize = (MAX_TEMPO - MIN_TEMPO + 1) as usize;
const MIN_ONSET_FRAMES: u64 = ANALYSIS_RATE as u64 / 6;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Estimate {
    pub bpm_tenths: u16,
    /// Timeline time of the first estimated grid beat in the retained audio clip.
    pub first: MediaTime,
    pub onsets: u64,
    /// Rhythmic consistency of the observed transients, not a probability of correctness.
    pub consistency: f64,
}

/// Decodes only the retained audio range on its caller's worker, honoring cancellation per PCM block.
pub fn analyze(
    path: &Path,
    clip: &AudioClip,
    cancellation: &Cancellation,
) -> Result<Estimate, String> {
    let map = clip.time_map().map_err(|error| error.to_string())?;
    let span = map.span();
    let mut cursor = sample_frame(span.start, ANALYSIS_RATE)?;
    let end = sample_frame(span.end, ANALYSIS_RATE)?;
    let mut decoder = ClipDecoder::open(path, clip, map, cursor, ANALYSIS_RATE, cancellation)?;
    let mut detector = Detector::default();
    let mut block = vec![[0.0; CHANNELS]; MAX_BLOCK_FRAMES];
    while cursor < end {
        cancellation.check()?;
        let count = ((end - cursor) as usize).min(block.len());
        decoder.read(&mut block[..count], cursor, cancellation)?;
        detector.push(&block[..count])?;
        cursor += count as u64;
    }
    decoder.finish(cancellation)?;
    let (bpm_tenths, offset, onsets, consistency) = detector.finish()?;
    let first = span
        .start
        .checked_add(sample_time(offset, ANALYSIS_RATE)?)
        .map_err(|error| error.to_string())?;
    Ok(Estimate {
        bpm_tenths,
        first,
        onsets,
        consistency,
    })
}

/// Produces a rational tempo grid in a half-open interval, without accumulating floating-point drift.
pub fn markers(
    first_id: Id,
    span: Span,
    bpm_tenths: u16,
    offset_millis: i64,
) -> Result<Vec<Marker>, StudioError> {
    span.validate()?;
    if !(MIN_TEMPO..=MAX_TEMPO).contains(&bpm_tenths) {
        return Err(StudioError::Invalid(
            "beat tempo is outside its supported range",
        ));
    }
    let minute = i128::from(60 * crate::time::MICROS_PER_SECOND * u64::from(TEMPO_SCALE));
    let tempo = i128::from(bpm_tenths);
    let origin = i128::from(span.start.0) + i128::from(offset_millis) * 1_000;
    // The beat index can be negative when the user places the first beat after the clip's start.
    let ceiling = |numerator: i128| -(-numerator).div_euclid(minute);
    let first = ceiling((i128::from(span.start.0) - origin) * tempo);
    let end = ceiling((i128::from(span.end.0) - origin) * tempo);
    let count = usize::try_from(end - first).map_err(|_| StudioError::TimeOverflow)?;
    crate::error::limit("tempo markers", count, crate::model::MAX_PROJECT_ITEMS)?;
    let mut output = Vec::with_capacity(count);
    for index in first..end {
        let at = origin + (index * minute).div_euclid(tempo);
        let id = first_id
            .0
            .checked_add(output.len() as u64)
            .ok_or(StudioError::IdentifiersExhausted)?;
        output.push(Marker {
            id: Id(id),
            at: MediaTime(at as u64),
            kind: MarkerKind::Beat,
            label: format!("Beat {}", output.len() + 1),
            confidence: Confidence::Correlated,
            origin: None,
        });
    }
    Ok(output)
}

struct Detector {
    frames: u64,
    window: u32,
    energy: f64,
    mean: f64,
    prior: f64,
    previous: f64,
    previous_threshold: f64,
    last: Option<u64>,
    onsets: u64,
    sine: [f64; TEMPOS],
    cosine: [f64; TEMPOS],
    intervals: [f64; TEMPOS],
}

impl Default for Detector {
    fn default() -> Self {
        Self {
            frames: 0,
            window: 0,
            energy: 0.0,
            mean: 0.0,
            prior: 0.0,
            previous: 0.0,
            previous_threshold: 0.0,
            last: None,
            onsets: 0,
            sine: [0.0; TEMPOS],
            cosine: [0.0; TEMPOS],
            intervals: [0.0; TEMPOS],
        }
    }
}

impl Detector {
    /// A fixed-memory envelope and tempo votes make analysis independent of recording duration.
    fn push(&mut self, frames: &[[f32; CHANNELS]]) -> Result<(), String> {
        for frame in frames {
            if frame.iter().any(|sample| !sample.is_finite()) {
                return Err("Beat analysis received invalid audio samples.".into());
            }
            self.energy += frame
                .iter()
                .map(|&value| f64::from(value).powi(2))
                .sum::<f64>()
                / CHANNELS as f64;
            self.window += 1;
            self.frames += 1;
            if self.window != WINDOW_FRAMES {
                continue;
            }
            let rms = (self.energy / f64::from(self.window)).sqrt();
            if self.previous > self.prior
                && self.previous >= rms
                && self.previous > self.previous_threshold
            {
                let at = self.frames.saturating_sub(u64::from(WINDOW_FRAMES) * 3 / 2);
                if self
                    .last
                    .is_none_or(|last| at.saturating_sub(last) >= MIN_ONSET_FRAMES)
                {
                    self.onset(at);
                }
            }
            let threshold = (self.mean * 1.65).max(0.004);
            self.mean += (rms - self.mean) * 0.04;
            self.prior = self.previous;
            self.previous = rms;
            self.previous_threshold = threshold;
            self.energy = 0.0;
            self.window = 0;
        }
        Ok(())
    }

    /// Every transient votes for a phase; inter-onset intervals resolve common half/double-tempo ambiguity.
    fn onset(&mut self, at: u64) {
        if let Some(last) = self.last {
            let interval = at.saturating_sub(last) as f64 / f64::from(ANALYSIS_RATE);
            let estimate = 60.0 / interval;
            for (index, vote) in self.intervals.iter_mut().enumerate() {
                let bpm = f64::from(MIN_BPM) + index as f64 / f64::from(TEMPO_SCALE);
                let distance = ((bpm - estimate) / estimate).abs();
                if distance < 0.04 {
                    *vote += 1.0 - distance / 0.04;
                }
            }
        }
        self.last = Some(at);
        self.onsets += 1;
        for index in 0..TEMPOS {
            let bpm = f64::from(MIN_BPM) + index as f64 / f64::from(TEMPO_SCALE);
            let phase = (at as f64 * bpm / (f64::from(ANALYSIS_RATE) * 60.0)).fract()
                * std::f64::consts::TAU;
            let (sine, cosine) = phase.sin_cos();
            self.sine[index] += sine;
            self.cosine[index] += cosine;
        }
    }

    /// A weak or silent source asks for manual tempo instead of inventing reliable beat evidence.
    fn finish(self) -> Result<(u16, u64, u64, f64), String> {
        if self.onsets < 4 {
            return Err(
                "Not enough rhythmic transients were found. Set the tempo and offset manually."
                    .into(),
            );
        }
        let score = |index: usize| {
            let consistency = self.sine[index].hypot(self.cosine[index]) / self.onsets as f64;
            consistency + self.intervals[index] / self.onsets as f64 * 0.5
        };
        let index = (0..TEMPOS)
            .max_by(|&a, &b| score(a).total_cmp(&score(b)))
            .expect("tempo registry is nonempty");
        let bpm_tenths = MIN_TEMPO + index as u16;
        let consistency = self.sine[index].hypot(self.cosine[index]) / self.onsets as f64;
        if consistency < 0.2 {
            return Err("The rhythm is irregular. Set a tempo and offset manually.".into());
        }
        let phase = self.sine[index]
            .atan2(self.cosine[index])
            .rem_euclid(std::f64::consts::TAU)
            / std::f64::consts::TAU;
        let offset = (phase * f64::from(ANALYSIS_RATE) * 60.0 * f64::from(TEMPO_SCALE)
            / f64::from(bpm_tenths))
        .round() as u64;
        Ok((bpm_tenths, offset, self.onsets, consistency))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tempo_grid_keeps_exact_phase_without_accumulated_rounding_or_end_markers() {
        let span = Span::new(MediaTime(2_000_000), MediaTime(5_000_000)).unwrap();
        let grid = markers(Id(10), span, 120 * TEMPO_SCALE, 125).unwrap();
        assert_eq!(
            grid.iter().map(|marker| marker.at.0).collect::<Vec<_>>(),
            vec![
                2_125_000, 2_625_000, 3_125_000, 3_625_000, 4_125_000, 4_625_000
            ]
        );
        let shifted = markers(Id(10), span, 120 * TEMPO_SCALE, -375).unwrap();
        assert_eq!(
            shifted.iter().map(|marker| marker.at).collect::<Vec<_>>(),
            grid.iter().map(|marker| marker.at).collect::<Vec<_>>()
        );
        assert!(markers(Id(u64::MAX), span, 120 * TEMPO_SCALE, 0).is_err());
    }

    #[test]
    fn streaming_rhythm_estimate_is_independent_of_pcm_block_boundaries() {
        let mut pcm = vec![[0.0; CHANNELS]; ANALYSIS_RATE as usize * 12];
        for beat in 0..24 {
            let at = beat * ANALYSIS_RATE as usize / 2 + 600;
            for frame in &mut pcm[at..at + 120] {
                *frame = [0.8; CHANNELS];
            }
        }
        let mut first = Detector::default();
        first.push(&pcm).unwrap();
        let mut second = Detector::default();
        for chunk in pcm.chunks(173) {
            second.push(chunk).unwrap();
        }
        let a = first.finish().unwrap();
        let b = second.finish().unwrap();
        assert_eq!(a, b);
        assert_eq!(a.0, 120 * TEMPO_SCALE);
        assert!(a.3 > 0.99);
        assert!(Detector::default().finish().is_err());
    }

    #[test]
    fn fractional_tempo_grid_uses_the_rational_clock_over_long_music() {
        let span = Span::new(MediaTime::ZERO, MediaTime(600_000_000)).unwrap();
        let grid = markers(Id(1), span, 1205, 0).unwrap();
        assert_eq!(grid.len(), 1205);
        for (index, marker) in grid.iter().enumerate() {
            assert_eq!(marker.at.0, index as u64 * 600_000_000 / 1205);
        }
    }

    #[test]
    fn a_long_tempo_grid_is_one_undoable_edit_and_preserves_other_markers() {
        use crate::{
            Project,
            edit::{Command, Editor},
        };
        let span = Span::new(MediaTime::ZERO, MediaTime(600_000_000)).unwrap();
        let mut project = Project::new("Rhythm");
        project.markers.push(Marker {
            id: Id(1),
            at: MediaTime(1_000_000),
            kind: MarkerKind::Manual,
            label: "Keep this".into(),
            confidence: Confidence::Observed,
            origin: None,
        });
        let mut editor = Editor::new(project).unwrap();
        let grid = markers(Id(2), span, 120 * TEMPO_SCALE, 0).unwrap();
        assert!(grid.len() > crate::edit::MAX_EDIT_COMMANDS);
        editor
            .apply(
                "Tempo grid",
                &[Command::SetBeatMarkers {
                    span,
                    markers: grid,
                }],
            )
            .unwrap();
        assert_eq!(editor.project().markers.len(), 1201);
        assert_eq!(editor.project().markers[0].label, "Keep this");
        editor.undo().unwrap();
        assert_eq!(editor.project().markers.len(), 1);
        editor.redo().unwrap();
        editor
            .apply(
                "Clear grid",
                &[Command::SetBeatMarkers {
                    span,
                    markers: Vec::new(),
                }],
            )
            .unwrap();
        assert_eq!(editor.project().markers.len(), 1);
        let bad = vec![editor.project().markers[0].clone()];
        assert!(
            editor
                .apply(
                    "Invalid beat grid",
                    &[Command::SetBeatMarkers { span, markers: bad }]
                )
                .is_err()
        );
        assert_eq!(editor.project().markers.len(), 1);
    }
}
