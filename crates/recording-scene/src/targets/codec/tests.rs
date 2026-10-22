use super::*;

/// The old envelope has no classification bytes; its label cannot supply those missing facts.
fn legacy_prefix() -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend(LEGACY_MAGIC);
    binary::u32(&mut bytes, 1).unwrap();
    for value in [7, 8, 1] {
        binary::u64(&mut bytes, value).unwrap();
    }
    for value in [0.0_f64; 9] {
        binary::u64(&mut bytes, value.to_bits()).unwrap();
    }
    binary::u32(&mut bytes, 0).unwrap();
    for _ in 0..3 {
        binary::u64(&mut bytes, 0).unwrap();
    }
    binary::u32(&mut bytes, 0).unwrap();
    binary::blob(&mut bytes, b"Player").unwrap();
    bytes
}

#[test]
fn old_target_envelopes_remain_unclassified_without_name_guesses() {
    let mut bytes = legacy_prefix();
    binary::u32(&mut bytes, 0).unwrap();
    let delta = read(&mut &bytes[..]).unwrap();
    let target = &delta.upserts[0];
    assert_eq!(target.kind, CameraTargetKind::Unknown);
    assert_eq!(target.entity_type, None);
    assert!(!target.is_player_subject(9));
    assert!(target.is_player_subject(7));
    assert_eq!(target.view, CameraTargetView::Unspecified);
}

#[test]
fn classified_older_tracks_preserve_type_without_inventing_eye_provenance() {
    let mut bytes = legacy_prefix();
    bytes[..CLASSIFIED_MAGIC.len()].copy_from_slice(&CLASSIFIED_MAGIC);
    binary::u32(&mut bytes, CameraTargetKind::Player.code()).unwrap();
    binary::blob(&mut bytes, b"minecraft:player").unwrap();
    binary::u32(&mut bytes, 0).unwrap();
    let target = read(&mut &bytes[..]).unwrap().upserts.remove(0);
    assert_eq!(target.kind, CameraTargetKind::Player);
    assert_eq!(target.view, CameraTargetView::Unspecified);
    assert!(target.pose.eye_offset.is_none());
}

#[test]
fn precise_eye_failure_survives_roundtrip_and_contradictory_provenance_is_rejected() {
    let mut bytes = legacy_prefix();
    binary::u32(&mut bytes, 0).unwrap();
    let mut delta = read(&mut &bytes[..]).unwrap();
    delta.upserts[0].view = CameraTargetView::Swimming;
    bytes.clear();
    write(&mut bytes, &delta).unwrap();
    let decoded = read(&mut &bytes[..]).unwrap();
    assert_eq!(decoded.upserts[0].view, CameraTargetView::Swimming);
    assert_eq!(
        decoded.upserts[0].eye_view_notice(),
        Some(CameraTargetView::Swimming.notice())
    );
    delta.upserts[0].view = CameraTargetView::LocalPresentation;
    assert!(write(&mut Vec::new(), &delta).is_err());
}

#[test]
fn classified_targets_round_trip_and_reject_unknown_class_codes() {
    let mut legacy = legacy_prefix();
    binary::u32(&mut legacy, 0).unwrap();
    let mut delta = read(&mut &legacy[..]).unwrap();
    delta.upserts[0].kind = CameraTargetKind::Entity;
    delta.upserts[0].entity_type = Some(Arc::from("minecraft:armor_stand"));
    let mut bytes = Vec::new();
    write(&mut bytes, &delta).unwrap();
    assert_eq!(read(&mut &bytes[..]).unwrap().upserts, delta.upserts);
    let mut malformed = legacy_prefix();
    malformed[..MAGIC.len()].copy_from_slice(&MAGIC);
    binary::u32(&mut malformed, u32::MAX).unwrap();
    binary::blob(&mut malformed, &[]).unwrap();
    binary::u32(&mut malformed, 0).unwrap();
    assert!(read(&mut &malformed[..]).is_err());
}
