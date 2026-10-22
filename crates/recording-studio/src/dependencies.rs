//! One cancellable source-identity admission precedes export; codecs and replay schemas retain their own owners.

use crate::{Project, export::Cancellation, model::MediaKind};
use std::{collections::BTreeSet, path::Path};

/// Checks used local sources once on a worker, including native audio sidecars; older unhashed media remains supported.
pub fn validate_sources(
    project: &Project,
    source_base: &Path,
    cancellation: &Cancellation,
) -> Result<(), String> {
    let duration = project.duration();
    let range = (duration != crate::MediaTime::ZERO || project.export.range.is_some())
        .then(|| {
            crate::export::range::FrameRange::new(
                project.export.fps,
                duration,
                project.export.range,
            )
        })
        .transpose()
        .map_err(|error| error.to_string())?;
    let coverage =
        crate::export::range::visual_coverage(project).map_err(|error| error.to_string())?;
    let visual: BTreeSet<_> = project
        .clips
        .iter()
        .filter(|clip| {
            coverage.get(&clip.id).is_some_and(|span| {
                range.is_some_and(|range| range.visual_intersects(*span).unwrap_or(false))
            })
        })
        .map(|clip| clip.source)
        .collect();
    let audio: BTreeSet<_> = project
        .audio
        .iter()
        .filter(|clip| {
            project.export.include_audio
                && !project.audio_muted(clip)
                && clip.gain > 0.0
                && clip.span().is_ok_and(|span| {
                    range.is_some_and(|range| {
                        range
                            .audio_window(span, crate::export::AUDIO_SAMPLE_RATE)
                            .is_ok_and(|window| window.is_some())
                    })
                })
        })
        .map(|clip| clip.source)
        .collect();
    for source in project
        .sources
        .iter()
        .filter(|source| visual.contains(&source.id) || audio.contains(&source.id))
    {
        cancellation.check()?;
        let path = if source.path.is_absolute() {
            source.path.clone()
        } else {
            source_base.join(&source.path)
        };
        let checked = (|| {
            if source.kind == MediaKind::Replay {
                if !path.is_dir() {
                    return Err("The recorded replay folder is missing. Restore its original folder before exporting.".into());
                }
                std::fs::read_dir(&path).map_err(|error| error.to_string())?;
                if audio.contains(&source.id) {
                    crate::media::check_file(&path.join(crate::capture::AUDIO_FILE))?;
                }
            } else {
                crate::media::check_file(&path)?;
                if let Some(expected) = source.sha256 {
                    let actual = crate::media::hash(&path, cancellation)?;
                    if actual != expected {
                        return Err("The source contents changed since import. Relink the matching original file, or import the replacement as new media.".into());
                    }
                }
            }
            Ok(())
        })();
        checked
            .map_err(|error: String| format!("{} ({}): {error}", source.name, path.display()))?;
    }
    cancellation.check()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Id, MediaTime, PlaybackRate, Span,
        model::{Clip, Framing, MediaSource},
    };

    #[test]
    fn range_admission_skips_sources_outside_output_but_keeps_dissolve_handles() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("present.mp4"), b"bounded source").unwrap();
        let mut project = Project::new("range dependency admission");
        for (id, path) in [(1, "missing.mp4"), (2, "present.mp4")] {
            project.sources.push(MediaSource {
                id: Id(id),
                name: path.into(),
                path: path.into(),
                kind: MediaKind::Video,
                duration: MediaTime(1_000_000),
                size: [16, 16],
                frame_rate: None,
                sha256: None,
            });
            project.clips.push(Clip {
                id: Id(id + 2),
                source: Id(id),
                source_range: Span::new(MediaTime::ZERO, MediaTime(1_000_000)).unwrap(),
                at: MediaTime((id - 1) * 1_000_000),
                rate: PlaybackRate::default(),
                speed_ramp: None,
                track: 0,
                framing: Framing::default(),
                enabled: true,
            });
        }
        project.export.range = Some(Span::new(MediaTime(1_000_000), MediaTime(1_050_000)).unwrap());
        let cancellation = Cancellation::default();
        validate_sources(&project, root.path(), &cancellation).unwrap();
        project.transitions.push(crate::model::Transition {
            id: Id(5),
            from: Id(3),
            to: Id(4),
            duration: MediaTime(200_000),
            kind: crate::model::TransitionKind::Crossfade,
        });
        assert!(validate_sources(&project, root.path(), &cancellation).is_err());
    }

    /// The first requested output picture is after a missing native prefix, following an FPS change.
    fn changed_grid_project() -> Project {
        let mut project = Project::new("changed output cadence");
        project.export.fps = crate::FrameRate {
            numerator: 24,
            denominator: 1,
        };
        project.export.range = Some(Span::new(MediaTime(33_333), MediaTime(100_000)).unwrap());
        for (id, kind, path) in [
            (1, MediaKind::Replay, "missing.replay"),
            (2, MediaKind::Video, "present.mp4"),
        ] {
            project.sources.push(MediaSource {
                id: Id(id),
                name: path.into(),
                path: path.into(),
                kind,
                duration: MediaTime(1_000_000),
                size: [16, 16],
                frame_rate: None,
                sha256: None,
            });
            project.clips.push(Clip {
                id: Id(id + 2),
                source: Id(id),
                source_range: Span::new(
                    MediaTime::ZERO,
                    MediaTime(if id == 1 { 40_000 } else { 960_000 }),
                )
                .unwrap(),
                at: MediaTime(if id == 1 { 0 } else { 40_000 }),
                rate: PlaybackRate::default(),
                speed_ramp: None,
                track: 0,
                framing: Framing::default(),
                enabled: true,
            });
        }
        project
    }

    #[test]
    fn missing_unsampled_prefix_is_skipped_but_its_sampled_dissolve_handle_is_required() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("present.mp4"), b"bounded source").unwrap();
        let mut project = changed_grid_project();
        project.validate().unwrap();
        let cancellation = Cancellation::default();
        validate_sources(&project, root.path(), &cancellation).unwrap();
        project.transitions.push(crate::model::Transition {
            id: Id(5),
            from: Id(3),
            to: Id(4),
            duration: MediaTime(2_000),
            kind: crate::model::TransitionKind::Crossfade,
        });
        validate_sources(&project, root.path(), &cancellation).unwrap();
        project.transitions[0].duration = MediaTime(4_000);
        assert!(validate_sources(&project, root.path(), &cancellation).is_err());
    }

    #[test]
    fn audio_sources_follow_pcm_intersection_instead_of_visual_or_requested_span_overlap() {
        use crate::model::{AudioClip, AudioRole};
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("present.mp4"), b"bounded source").unwrap();
        let mut project = changed_grid_project();
        project.sources.push(MediaSource {
            id: Id(5),
            name: "missing audio".into(),
            path: "missing.wav".into(),
            kind: MediaKind::Audio,
            duration: MediaTime(1_000_000),
            size: [0, 0],
            frame_rate: None,
            sha256: None,
        });
        project.audio.push(AudioClip {
            id: Id(6),
            source: Id(5),
            linked_clip: None,
            source_range: Span::new(MediaTime::ZERO, MediaTime(5_000)).unwrap(),
            at: MediaTime(35_000),
            rate: PlaybackRate::default(),
            speed_ramp: None,
            role: AudioRole::Music,
            gain: 1.0,
            fade_in: MediaTime::ZERO,
            fade_out: MediaTime::ZERO,
            muted: false,
        });
        project.validate().unwrap();
        let cancellation = Cancellation::default();
        validate_sources(&project, root.path(), &cancellation).unwrap();
        project.audio[0].at = MediaTime(44_001);
        project.audio[0].source_range.end = MediaTime(1);
        validate_sources(&project, root.path(), &cancellation).unwrap();
        project.audio[0].at = MediaTime(45_000);
        project.audio[0].source_range.end = MediaTime(5_000);
        assert!(validate_sources(&project, root.path(), &cancellation).is_err());
        project.audio[0].muted = true;
        validate_sources(&project, root.path(), &cancellation).unwrap();
        project.audio[0].muted = false;
        project.audio[0].source = Id(1);
        project.audio[0].role = AudioRole::Game;
        std::fs::create_dir(root.path().join("missing.replay")).unwrap();
        let error = validate_sources(&project, root.path(), &cancellation).unwrap_err();
        assert!(error.contains(crate::capture::AUDIO_FILE));
        project.export.include_audio = false;
        validate_sources(&project, root.path(), &cancellation).unwrap();
    }

    #[test]
    fn one_admission_rejects_changed_hashed_media_and_keeps_legacy_unhashed_media_usable() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("source.bin");
        std::fs::write(&path, b"original media").unwrap();
        let cancellation = Cancellation::default();
        let mut project = Project::new("identity");
        project.sources.push(MediaSource {
            id: Id(1),
            name: "source".into(),
            path: "source.bin".into(),
            kind: MediaKind::Image,
            duration: MediaTime(1_000_000),
            size: [1, 1],
            frame_rate: None,
            sha256: Some(crate::media::hash(&path, &cancellation).unwrap()),
        });
        project.clips.push(Clip {
            id: Id(2),
            source: Id(1),
            source_range: Span::new(MediaTime::ZERO, MediaTime(1_000_000)).unwrap(),
            at: MediaTime::ZERO,
            rate: PlaybackRate::default(),
            speed_ramp: None,
            track: 0,
            framing: Framing::default(),
            enabled: true,
        });
        validate_sources(&project, root.path(), &cancellation).unwrap();
        std::fs::write(&path, b"replaced media").unwrap();
        assert!(
            validate_sources(&project, root.path(), &cancellation)
                .unwrap_err()
                .contains("changed since import")
        );
        project.sources[0].sha256 = None;
        validate_sources(&project, root.path(), &cancellation).unwrap();
        project.clips[0].enabled = false;
        std::fs::remove_file(path).unwrap();
        validate_sources(&project, root.path(), &cancellation).unwrap();
    }
}
