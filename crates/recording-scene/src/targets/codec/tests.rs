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
