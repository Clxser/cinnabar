//! Owns one bounded journal and hardware-preferred video encode; frame adapters supply all time.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use super::*;
use crate::{
    archive::{
        ArchiveError, RecordInput, RecordKind, Recording, RecordingConfig, RecordingInfo,
        RecordingState, SCENE_CHECKPOINT_INTERVAL,
    },
    automation::{CombatDetector, GameplayEvent, GameplayEventKind, Highlight},
    camera::CameraPose,
    video::{EncodeOptions, EncodingSpeed, FrameSink, Recorder, Summary, VideoSettings},
};

pub struct CaptureSession {
    directory: PathBuf,
    info: RecordingInfo,
    settings: CaptureSettings,
    journal: Option<Recording>,
    video: Option<Recorder>,
    detector: CombatDetector,
    next_event: u64,
    highlights: Vec<Highlight>,
    observations: Vec<Highlight>,
    omitted: u64,
    scene_tracks: Vec<CaptureTrack>,
    camera_checkpoint: MediaTime,
}

pub struct CaptureSummary {
    pub directory: PathBuf,
    pub video: Option<Summary>,
    pub manifest: CaptureManifest,
}

impl CaptureSession {
    /// Creates files and probes the encoder on a startup worker, before the live capture clock starts.
    /// The archive reserves its directory before the video encoder is allowed to create media there.
    pub fn create(
        root: &Path,
        mut info: RecordingInfo,
        settings: CaptureSettings,
    ) -> Result<Self, String> {
        settings.validate()?;
        settings.size.fit(info.viewport)?;
        info.scene_schema = POV_SCENE_SCHEMA.into();
        if settings.mode.records_video() {
            crate::video::find_ffmpeg()?;
        }
        std::fs::create_dir_all(root).map_err(|error| error.to_string())?;
        let mut opened = None;
        for ordinal in 0..1024 {
            let directory = root.join(format!(
                "Recording {}-{ordinal}.{}",
                info.created_unix_ms,
                crate::archive::RECORDING_EXTENSION
            ));
            match Recording::start_ready(
                directory.clone(),
                info.clone(),
                RecordingConfig::default(),
            ) {
                Ok(journal) => {
                    opened = Some((directory, journal));
                    break;
                }
                Err(ArchiveError::Io { source, .. })
                    if source.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.to_string()),
            }
        }
        let (directory, mut journal) =
            opened.ok_or("Recording names are exhausted; choose another studio folder.")?;
        for (kind, payload) in [
            (RecordKind::CheckpointStart, Arc::<[u8]>::from([])),
            (
                RecordKind::CheckpointPart,
                serde_json::to_vec(&PovCheckpoint {
                    version: POV_CHECKPOINT_VERSION,
                    settings,
                    camera: None,
                })
                .map_err(|error| error.to_string())?
                .into(),
            ),
            (RecordKind::CheckpointEnd, Arc::<[u8]>::from([])),
        ] {
            journal
                .push(RecordInput {
                    at: MediaTime::ZERO,
                    kind,
                    payload,
                })
                .map_err(|error| error.to_string())?;
        }
        let video = if settings.mode.records_video() {
            Some(Recorder::start_prepared(
                VideoSettings {
                    path: directory.join(VIDEO_FILE),
                    fps: settings.fps,
                    codec: settings.codec,
                },
                EncodeOptions {
                    encoder: settings.encoder,
                    quality: Some(settings.quality),
                    speed: EncodingSpeed::Live,
                },
            )?)
        } else {
            None
        };
        Ok(Self {
            directory,
            settings,
            journal: Some(journal),
            video,
            detector: CombatDetector::new(info.local_actor),
            next_event: 1,
            highlights: Vec::new(),
            observations: Vec::with_capacity(2),
            omitted: 0,
            info,
            scene_tracks: Vec::new(),
            camera_checkpoint: MediaTime::ZERO,
        })
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }
    /// Supplies already admitted metadata to optional scene workers without a frame-thread file read.
    pub fn info(&self) -> &RecordingInfo {
        &self.info
    }
    pub fn settings(&self) -> CaptureSettings {
        self.settings
    }
    /// Video can be smaller than the viewport retained by native scene and HUD tracks.
    pub fn video_size(&self) -> [u32; 2] {
        self.settings
            .size
            .fit(self.info.viewport)
            .expect("capture size was admitted before creating the session")
    }
    /// Records a scene owner's final status before the manifest is published with finished POV media.
    pub fn scene_track(&mut self, track: CaptureTrack) -> Result<(), String> {
        track.validate()?;
        if self.scene_tracks.len() >= MAX_CAPTURE_TRACKS
            || self
                .scene_tracks
                .iter()
                .any(|old| old.schema == track.schema || old.directory == track.directory)
        {
            return Err("Duplicate or excessive captured scene tracks.".into());
        }
        self.scene_tracks.push(track);
        Ok(())
    }
    pub fn sink(&self) -> Option<FrameSink> {
        self.video.as_ref().map(Recorder::sink)
    }

    /// Records actual camera metadata; the POV package deliberately does not claim 3D reconstruction.
    pub fn camera(&mut self, at: MediaTime, pose: CameraPose) -> Result<(), String> {
        pose.validate().map_err(|error| error.to_string())?;
        if at.0.saturating_sub(self.camera_checkpoint.0) >= SCENE_CHECKPOINT_INTERVAL.0 {
            let journal = self.journal.as_mut().ok_or("Recording is finishing.")?;
            journal
                .push(RecordInput {
                    at,
                    kind: RecordKind::CheckpointStart,
                    payload: Arc::from([]),
                })
                .map_err(|error| error.to_string())?;
            self.encoded(
                at,
                RecordKind::CheckpointPart,
                std::mem::size_of::<PovCheckpoint>(),
                PovCheckpoint {
                    version: POV_CHECKPOINT_VERSION,
                    settings: self.settings,
                    camera: Some(pose),
                },
            )?;
            self.journal
                .as_mut()
                .ok_or("Recording is finishing.")?
                .push(RecordInput {
                    at,
                    kind: RecordKind::CheckpointEnd,
                    payload: Arc::from([]),
                })
                .map_err(|error| error.to_string())?;
            self.camera_checkpoint = at;
            return Ok(());
        }
        self.encoded(
            at,
            RecordKind::Camera,
            std::mem::size_of::<CameraPose>(),
            pose,
        )
    }

    /// Observations retain confidence and provenance before any editable effects are proposed.
    pub fn event(&mut self, at: MediaTime, kind: GameplayEventKind) -> Result<(), String> {
        if let GameplayEventKind::Bookmark { label } = &kind
            && label.len() > 256
        {
            return Err("A recording bookmark must be at most 256 bytes.".into());
        }
        let event = GameplayEvent {
            sequence: self.next_event,
            at,
            kind,
        };
        self.next_event = self
            .next_event
            .checked_add(1)
            .ok_or("Recording event sequence was exhausted.")?;
        self.observations.clear();
        self.detector
            .observe(&event, &mut self.observations)
            .map_err(|error| error.to_string())?;
        for observation in self.observations.drain(..) {
            if self.highlights.len() < MAX_CAPTURE_HIGHLIGHTS {
                self.highlights.push(observation);
            } else {
                self.omitted += 1;
            }
        }
        self.encoded(
            at,
            RecordKind::Gameplay,
            std::mem::size_of::<GameplayEvent>() + 256,
            event,
        )
    }

    /// Serialization is deferred; the live thread transfers small owned values through bounded admission.
    fn encoded<T: serde::Serialize + Send + 'static>(
        &mut self,
        at: MediaTime,
        kind: RecordKind,
        retained: usize,
        value: T,
    ) -> Result<(), String> {
        self.journal
            .as_mut()
            .ok_or("Recording is finishing.")?
            .push_prepared(at, kind, retained, move |out| {
                serde_json::to_writer(out, &value).map_err(ArchiveError::from)
            })
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    /// Surfaces worker failures without joining or touching files on the live frame.
    pub fn error(&self) -> Option<String> {
        if self.video.as_ref().is_some_and(Recorder::worker_finished) {
            return Some("The video encoder stopped; finalizing the captured prefix.".into());
        }
        self.journal.as_ref()?.task().progress()?.error
    }

    /// Missing server notices reset correlations rather than claiming a complete combo sequence.
    pub fn missed_observations(&mut self, count: u64, local_actor: u64) {
        self.omitted = self.omitted.saturating_add(count);
        self.detector = CombatDetector::new(local_actor);
    }

    /// Finalizes only on an IO worker after the frame adapter has drained its sole GPU readback.
    pub fn finish(
        mut self,
        duration: MediaTime,
        audio: Option<&Path>,
        missed_video_frames: u64,
        dropped_audio_samples: u64,
    ) -> Result<CaptureSummary, String> {
        let expected_frames = self
            .settings
            .fps
            .frames_covering(duration)
            .map_err(|error| error.to_string())?;
        if let Some(recorder) = &self.video {
            recorder.set_final_frame_count(expected_frames);
        }
        let task = self
            .journal
            .take()
            .ok_or("Capture journal already finished.")?
            .finish(duration)
            .map_err(|error| error.to_string())?;
        let video = self
            .video
            .take()
            .map(|recorder| recorder.finish(audio))
            .transpose()?;
        let progress = task.wait();
        if progress.state != RecordingState::Complete {
            return Err(progress
                .error
                .unwrap_or_else(|| "Capture metadata did not finish writing.".into()));
        }
        let manifest = CaptureManifest {
            version: MANIFEST_VERSION,
            settings: self.settings,
            frames: video.as_ref().map_or(expected_frames, |video| video.frames),
            size: match &video {
                Some(video) => video
                    .size
                    .ok_or("No rendered video frames were captured.")?,
                None => self.video_size(),
            },
            duration: match &video {
                Some(video) => self
                    .settings
                    .fps
                    .frame_time(video.frames)
                    .map_err(|error| error.to_string())?,
                None => duration,
            },
            game_audio: audio.is_some(),
            highlights: self.highlights,
            omitted_highlights: self.omitted,
            missed_video_frames,
            dropped_audio_samples,
            scene_tracks: self.scene_tracks,
        };
        manifest.write(&self.directory)?;
        Ok(CaptureSummary {
            directory: self.directory,
            video,
            manifest,
        })
    }
}
