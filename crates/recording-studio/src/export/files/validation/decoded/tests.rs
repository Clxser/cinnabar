use super::*;

/// A tiny original synthetic movie describes the exact output contract without external recordings.
fn expected() -> Expected {
    Expected {
        size: [16, 16],
        fps: FrameRate {
            numerator: 60,
            denominator: 1,
        },
        frames: 6,
        audio: false,
    }
}

/// FFprobe fields deliberately contain string numbers, as its JSON writer produces.
fn metadata() -> serde_json::Value {
    serde_json::json!({"streams": [{
        "codec_type": "video", "width": 16, "height": 16,
        "avg_frame_rate": "60/1", "duration": "0.100000",
        "start_time": "0.000000", "nb_frames": "6"
    }]})
}

#[test]
fn export_stream_extent_cadence_frame_count_and_zero_based_timing_must_match() {
    assert!(validate_metadata(&serde_json::to_vec(&metadata()).unwrap(), expected()).is_ok());
    for (field, value) in [
        ("width", serde_json::json!(32)),
        ("avg_frame_rate", serde_json::json!("30/1")),
        ("avg_frame_rate", serde_json::json!("60/0")),
        ("nb_frames", serde_json::json!("5")),
        ("duration", serde_json::json!("0.090000")),
        ("duration", serde_json::json!("NaN")),
        ("start_time", serde_json::json!("1.000000")),
    ] {
        let mut value_copy = metadata();
        value_copy["streams"][0][field] = value;
        assert!(
            validate_metadata(&serde_json::to_vec(&value_copy).unwrap(), expected()).is_err(),
            "{field}"
        );
    }
    let mut equivalent_rate = metadata();
    equivalent_rate["streams"][0]["avg_frame_rate"] = serde_json::json!("120/2");
    assert!(validate_metadata(&serde_json::to_vec(&equivalent_rate).unwrap(), expected()).is_ok());
}

#[test]
fn audio_presence_format_and_sync_are_verified_with_only_codec_padding_tolerance() {
    let mut probe = metadata();
    probe["streams"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
        "codec_type": "audio", "sample_rate": AUDIO_SAMPLE_RATE.to_string(), "channels": crate::audio::CHANNELS,
            "duration": "0.100000", "start_time": "0.000000"
        }));
    let expected_audio = Expected {
        audio: true,
        ..expected()
    };
    let bytes = serde_json::to_vec(&probe).unwrap();
    assert!(validate_metadata(&bytes, expected_audio).is_ok());
    assert!(validate_metadata(&bytes, expected()).is_err());
    assert!(validate_metadata(&serde_json::to_vec(&metadata()).unwrap(), expected_audio).is_err());
    for (field, value) in [
        ("duration", serde_json::json!("0.300000")),
        ("start_time", serde_json::json!("0.010000")),
        (
            "sample_rate",
            serde_json::json!((AUDIO_SAMPLE_RATE + 1).to_string()),
        ),
        ("channels", serde_json::json!(1)),
    ] {
        let mut changed = probe.clone();
        changed["streams"][1][field] = value;
        assert!(
            validate_metadata(&serde_json::to_vec(&changed).unwrap(), expected_audio).is_err(),
            "{field}"
        );
    }
}

#[test]
fn successful_decode_requires_every_frame_and_a_terminal_progress_record() {
    let mut progress = Progress::default();
    progress.observe(b"frame=3\n", 6).unwrap();
    progress.observe(b"progress=continue\n", 6).unwrap();
    assert!(progress.finish(6).is_err());
    progress.observe(b"frame=6\n", 6).unwrap();
    assert!(progress.finish(6).is_err());
    progress.observe(b"progress=end\n", 6).unwrap();
    progress.finish(6).unwrap();
    assert!(progress.observe(b"frame=7\n", 6).is_err());
    assert!(progress.observe(b"frame=5\n", 6).is_err());
    assert!(progress.observe(b"frame=unknown\n", 6).is_err());
}

#[test]
fn verification_decodes_original_synthetic_video_and_rejects_damaged_payload() {
    let Ok(ffmpeg) = crate::video::find_ffmpeg() else {
        eprintln!("missing-fixture: FFmpeg executable for export payload validation");
        return;
    };
    if crate::media::probe_executable(&ffmpeg).is_err() {
        eprintln!("missing-fixture: FFprobe executable for export payload validation");
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("synthetic.mp4");
    let mut command = Command::new(ffmpeg);
    command
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-f",
            "lavfi",
            "-i",
            "color=c=red:s=16x16:r=60",
            "-frames:v",
            "6",
            "-an",
            "-c:v",
            "mpeg4",
        ])
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    crate::media_process::hide_console(&mut command);
    assert!(
        command.status().unwrap().success(),
        "synthetic FFmpeg fixture creation failed"
    );
    let cancellation = Cancellation::default();
    verify_media(&path, [16, 16], expected().fps, 6, false, &cancellation).unwrap();
    assert!(verify_media(&path, [16, 16], expected().fps, 5, false, &cancellation).is_err());
    let mut bytes = std::fs::read(&path).unwrap();
    let mut at = 0;
    let mut damaged = false;
    while at + 8 <= bytes.len() {
        let size = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        assert!(size >= 8 && at + size <= bytes.len());
        if &bytes[at + 4..at + 8] == b"mdat" {
            bytes[at + 8..at + size].fill(0);
            damaged = true;
        }
        at += size;
    }
    assert!(damaged);
    std::fs::write(&path, bytes).unwrap();
    super::super::validate(&path, u64::MAX, &cancellation).unwrap();
    assert!(verify_media(&path, [16, 16], expected().fps, 6, false, &cancellation).is_err());
}
