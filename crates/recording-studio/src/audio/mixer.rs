//! Forward audio blocks retain only currently audible clip decoders and one bounded scratch block.

use super::*;
use crate::{Project, Span, export::Cancellation, retime::TimeMap};
use std::{collections::BTreeMap, path::Path, sync::Arc};

struct Track {
    index: usize,
    map: TimeMap,
    span: Span,
    start: u64,
    end: u64,
    path: PathBuf,
}

pub struct Mixer {
    project: Arc<Project>,
    tracks: Vec<Track>,
    decoders: BTreeMap<usize, ClipDecoder>,
    active: Vec<usize>,
    scratch: Vec<[f32; CHANNELS]>,
    rate: u32,
    next: u64,
    upcoming: usize,
    end: u64,
}

impl Mixer {
    /// Project validation and path resolution happen once on the owning worker, never per PCM sample.
    pub fn new(project: Arc<Project>, base: &Path, rate: u32, start: u64) -> Result<Self, String> {
        project.validate().map_err(|error| error.to_string())?;
        let end = sample_frame(project.duration(), rate)?;
        if start > end {
            return Err("The Studio mixer starts beyond its timeline.".into());
        }
        let paths: BTreeMap<_, _> = project
            .sources
            .iter()
            .map(|source| (source.id, source_path(source, base)))
            .collect();
        let mut tracks = Vec::with_capacity(project.audio.len());
        for (index, clip) in project.audio.iter().enumerate() {
            if project.audio_muted(clip) || clip.gain == 0.0 {
                continue;
            }
            let map = clip.time_map().map_err(|error| error.to_string())?;
            let span = map.span();
            tracks.push(Track {
                index,
                start: sample_frame(span.start, rate)?,
                end: sample_frame(span.end, rate)?,
                span,
                map,
                path: paths
                    .get(&clip.source)
                    .ok_or("An audio source is missing.")?
                    .clone(),
            });
        }
        tracks.sort_by_key(|track| (track.start, track.index));
        let upcoming = tracks.partition_point(|track| track.start <= start);
        let active: Vec<_> = (0..upcoming)
            .filter(|&index| tracks[index].end > start)
            .take(MAX_PREVIEW_STREAMS + 1)
            .collect();
        if active.len() > MAX_PREVIEW_STREAMS {
            return Err(stream_budget_error());
        }
        Ok(Self {
            project,
            tracks,
            decoders: BTreeMap::new(),
            active,
            scratch: Vec::new(),
            rate,
            next: start,
            upcoming,
            end,
        })
    }

    pub fn end_frame(&self) -> u64 {
        self.end
    }

    /// Final PCM already owns its data; ended sources can retire before the device drains the queue.
    pub fn retire_decoders(&mut self) {
        for (_, decoder) in std::mem::take(&mut self.decoders) {
            decoder.retire();
        }
    }

    /// Every clip uses the absolute shared clock; gain, mute and fades never change another track's time.
    pub fn read(
        &mut self,
        output: &mut [[f32; CHANNELS]],
        start: u64,
        cancellation: &Cancellation,
    ) -> Result<(), String> {
        if start != self.next || output.len() > MAX_BLOCK_FRAMES {
            return Err("The Studio mixer requires bounded contiguous PCM blocks.".into());
        }
        let end = start
            .checked_add(output.len() as u64)
            .filter(|end| *end <= self.end)
            .ok_or("The Studio mix block exceeds its timeline.")?;
        cancellation.check()?;
        output.fill([0.0; CHANNELS]);
        self.active.retain(|&index| self.tracks[index].end > start);
        while self.upcoming < self.tracks.len() && self.tracks[self.upcoming].start < end {
            self.active.push(self.upcoming);
            self.upcoming += 1;
            if self.active.len() > MAX_PREVIEW_STREAMS {
                return Err(stream_budget_error());
            }
        }
        self.active.sort_unstable();
        self.active.dedup();
        if self.active.len() > MAX_PREVIEW_STREAMS {
            return Err(stream_budget_error());
        }
        let retired: Vec<_> = self
            .decoders
            .keys()
            .copied()
            .filter(|index| !self.active.contains(index))
            .collect();
        for index in retired {
            self.decoders
                .remove(&index)
                .expect("retiring audio decoder")
                .retire();
        }
        for &index in &self.active {
            cancellation.check()?;
            let track = &self.tracks[index];
            let begin = start.max(track.start);
            let until = end.min(track.end);
            if begin >= until {
                continue;
            }
            if let std::collections::btree_map::Entry::Vacant(entry) = self.decoders.entry(index) {
                entry.insert(ClipDecoder::open(
                    &track.path,
                    &self.project.audio[track.index],
                    track.map.clone(),
                    begin,
                    self.rate,
                    cancellation,
                )?);
            }
            self.scratch
                .resize((until - begin) as usize, [0.0; CHANNELS]);
            self.decoders
                .get_mut(&index)
                .expect("active audio decoder")
                .read(&mut self.scratch, begin, cancellation)?;
            add(
                &self.project.audio[track.index],
                track.span,
                begin,
                self.rate,
                &self.scratch,
                &mut output[(begin - start) as usize..(until - start) as usize],
            )?;
        }
        self.next = end;
        Ok(())
    }
}

/// Admission reports the fixed concurrent decoder budget before opening another media process.
fn stream_budget_error() -> String {
    format!(
        "Studio audio exceeds its {MAX_PREVIEW_STREAMS}-stream preview budget. Mute or consolidate audio tracks."
    )
}

/// Export and preview share the exact finite gain/fade mix rather than rebuilding time maps per sample.
pub fn add(
    clip: &crate::model::AudioClip,
    span: Span,
    start: u64,
    rate: u32,
    source: &[[f32; CHANNELS]],
    target: &mut [[f32; CHANNELS]],
) -> Result<(), String> {
    if source.len() != target.len()
        || source.len() > MAX_BLOCK_FRAMES
        || start.checked_add(source.len() as u64).is_none()
    {
        return Err("The audio mix block has inconsistent bounds.".into());
    }
    sample_time(start, rate)?;
    for (index, (input, output)) in source.iter().zip(target).enumerate() {
        let gain = clip.gain_in_span(sample_time(start + index as u64, rate)?, span) as f32;
        for channel in 0..CHANNELS {
            let value = output[channel] + input[channel] * gain;
            if !input[channel].is_finite() || !value.is_finite() {
                return Err("Audio contains non-finite mixed samples.".into());
            }
            output[channel] = value;
        }
    }
    Ok(())
}
