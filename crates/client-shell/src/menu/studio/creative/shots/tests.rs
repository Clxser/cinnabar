use super::*;
use recording_studio::camera::coverage::TargetCoverage;
use recording_studio::camera::{ActorPose, CameraTarget, CameraTargetKind};

/// Subject fixtures carry a real spawn identity while source media remains metadata-only.
fn target() -> CameraTarget {
    CameraTarget {
        runtime_id: 7,
        unique_id: 11,
        spawn_revision: 1,
        name: "Subject".into(),
        kind: CameraTargetKind::Player,
        entity_type: Some("minecraft:player".into()),
        view: Default::default(),
        teleported: false,
        pose: ActorPose {
            position: [0.0, 64.0, 4.0],
            velocity: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
            eye_offset: Some([0.0, 1.62, 0.0]),
        },
    }
}

/// A pending proof is injected at its publication boundary, avoiding threads, files and timing assumptions.
fn pending(
    rate: PlaybackRate,
) -> (
    Studio,
    crossbeam_channel::Sender<coverage::TargetCoverageResult>,
    Cancellation,
) {
    let mut project = Project::new("Synthetic native edit");
    project.sources.push(MediaSource {
        id: Id(1),
        name: "Native take".into(),
        path: "native".into(),
        kind: MediaKind::Replay,
        duration: MediaTime(8_000_000),
        size: [1280, 720],
        frame_rate: None,
        sha256: None,
    });
    project.clips.push(Clip {
        id: Id(2),
        source: Id(1),
        source_range: Span::new(MediaTime::ZERO, MediaTime(8_000_000)).unwrap(),
        at: MediaTime::ZERO,
        rate,
        speed_ramp: None,
        track: 0,
        framing: Framing::default(),
        enabled: true,
    });
    let mut studio = Studio::new(PathBuf::new());
    studio.install(Editor::new(project).unwrap(), None);
    let project = Arc::clone(studio.project().unwrap());
    let subject = target();
    let span = Span::new(MediaTime::ZERO, MediaTime(4_000_000)).unwrap();
    let shots: Arc<[CameraShot]> = CameraPreset::ALL
        .iter()
        .enumerate()
        .map(|(index, preset)| {
            CameraShot::cinematic_preset(
                Id(3 + index as u64),
                span,
                *preset,
                CameraPose::default(),
                Some(&subject),
            )
            .unwrap()
        })
        .collect::<Vec<_>>()
        .into();
    let framing = shots
        .iter()
        .map(|shot| {
            shot.sample(MediaTime::ZERO, |_| Some(subject.pose))
                .unwrap()
        })
        .collect();
    let cancellation = Cancellation::default();
    let (sender, result) = crossbeam_channel::bounded(1);
    Arc::make_mut(&mut studio.view).camera_target = Some(subject.reference());
    studio.creative.shot_analysis = Some(ShotJob {
        draft: ShotDraft {
            document: studio.document,
            project: Arc::clone(&project),
            shots,
            framing,
            verified_target: None,
        },
        epoch: studio.transport.epoch(),
        position: studio.transport.position(),
        camera_view: None,
        selected_target: Some(subject.reference()),
        map: project.clips[0].time_map().unwrap(),
        source_duration: project.sources[0].duration,
        cancellation: cancellation.clone(),
        result,
    });
    Arc::make_mut(&mut studio.view)
        .workflow
        .creative
        .shot_analyzing = true;
    (studio, sender, cancellation)
}

/// Proofs cover a canonical lifetime and a source interval, never output seconds inferred from FPS.
fn proof(end: u64) -> TargetCoverage {
    TargetCoverage {
        source_range: Span::new(MediaTime::ZERO, MediaTime(end)).unwrap(),
        target: target().reference(),
    }
}

#[test]
fn verified_lifetime_shortens_retimed_subject_shots_and_preserves_unbound_paths() {
    let (mut studio, sender, _) = pending(PlaybackRate {
        numerator: 2,
        denominator: 1,
    });
    sender.send(Ok(Some(proof(3_000_000)))).unwrap();
    studio.poll_shot_coverage();
    let shots = &studio.view.workflow.creative.shots;
    assert!(shots.iter().any(|shot| shot.target_reference().is_some()));
    for shot in shots.iter() {
        assert_eq!(
            shot.span.end,
            MediaTime(if shot.target_reference().is_some() {
                1_500_000
            } else {
                4_000_000
            })
        );
    }
    let subject_index = shots
        .iter()
        .position(|shot| shot.target_reference().is_some())
        .unwrap();
    let offered = studio.suggested_shot(subject_index).unwrap();
    studio.apply_suggested_shot(subject_index).unwrap();
    assert_eq!(studio.project().unwrap().shots, vec![offered]);
    studio.editor.as_mut().unwrap().undo().unwrap();
    assert!(studio.project().unwrap().shots.is_empty());
}

#[test]
fn missing_or_different_lifetimes_cannot_publish_subject_alternatives() {
    for proof in [
        None,
        Some(TargetCoverage {
            target: recording_studio::camera::CameraTargetRef {
                lifetime: Some(recording_studio::camera::TargetLifetime {
                    unique_id: 11,
                    spawn_revision: 2,
                }),
                ..target().reference()
            },
            ..proof(4_000_000)
        }),
        Some(proof(50_000)),
    ] {
        let (mut studio, sender, _) = pending(PlaybackRate::default());
        sender.send(Ok(proof)).unwrap();
        studio.poll_shot_coverage();
        assert!(!studio.view.workflow.creative.shots.is_empty());
        assert!(
            studio
                .view
                .workflow
                .creative
                .shots
                .iter()
                .all(|shot| shot.target_reference().is_none())
        );
    }
}

#[test]
fn seek_retime_and_source_relink_cancel_proof_before_any_late_result_is_published() {
    for mutation in 0..6 {
        let (mut studio, sender, cancellation) = pending(PlaybackRate::default());
        sender.send(Ok(Some(proof(4_000_000)))).unwrap();
        match mutation {
            0 => {
                studio
                    .transport
                    .seek(MediaTime(1_000_000), MediaTime(8_000_000));
            }
            1 => {
                let mut clip = studio.project().unwrap().clips[0].clone();
                clip.rate.numerator = 2;
                studio
                    .edit("Retime clip", &[Command::ReplaceClip { clip }])
                    .unwrap();
            }
            2 => {
                let mut source = studio.project().unwrap().sources[0].clone();
                source.path = "different-take".into();
                studio
                    .edit("Relink source", &[Command::ReplaceSource { source }])
                    .unwrap();
            }
            3 => {
                Arc::make_mut(&mut studio.view).camera_target = None;
            }
            4 => {
                Arc::make_mut(&mut studio.view).camera_workspace.view = Some(CameraView {
                    pose: CameraPose::default(),
                    revision: 1,
                });
            }
            _ => {
                studio.transport.play(MediaTime(8_000_000));
                studio
                    .transport
                    .advance(MediaTime(100_000), MediaTime(8_000_000));
            }
        }
        studio.poll_shot_coverage();
        assert!(cancellation.cancelled());
        assert!(studio.creative.shot_analysis.is_none());
        assert!(studio.view.workflow.creative.shots.is_empty());
        assert!(studio.project().unwrap().shots.is_empty());
    }
}

#[test]
fn cancellation_and_document_replacement_disconnect_stale_completions_without_waiting() {
    let (mut studio, sender, cancellation) = pending(PlaybackRate::default());
    studio.cancel_shot_analysis();
    assert!(cancellation.cancelled());
    assert!(sender.try_send(Ok(Some(proof(4_000_000)))).is_err());
    let (mut studio, sender, cancellation) = pending(PlaybackRate::default());
    studio.install(
        Editor::new(Project::new("Different project")).unwrap(),
        None,
    );
    assert!(cancellation.cancelled());
    assert!(sender.try_send(Ok(Some(proof(4_000_000)))).is_err());
    studio.poll_shot_coverage();
    assert!(studio.view.workflow.creative.shots.is_empty());
}
