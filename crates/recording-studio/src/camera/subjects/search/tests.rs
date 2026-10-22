use super::super::{CameraTargetKind, tests::target};
use super::*;

#[test]
fn substring_search_uses_unicode_lowercase_and_repeated_prefixes_without_changing_class_admission()
{
    let mut player = target(CameraTargetKind::Player);
    player.name = Arc::from("ÉLÉONORE abababac");
    assert!(
        CameraSubjectSearch::new("  élé  ")
            .unwrap()
            .matches(&player)
    );
    assert!(CameraSubjectSearch::new("abac").unwrap().matches(&player));
    assert!(!CameraSubjectSearch::new("abacx").unwrap().matches(&player));
    let entity = target(CameraTargetKind::Entity);
    let search = CameraSubjectSearch::new("player").unwrap();
    assert!(!search.includes(CameraSubjectFilter::Players, &entity, entity.runtime_id));
    assert!(search.includes(CameraSubjectFilter::All, &entity, 0));
    let unknown = target(CameraTargetKind::Unknown);
    assert!(!search.includes(CameraSubjectFilter::Players, &unknown, 0));
    assert!(search.includes(CameraSubjectFilter::Players, &unknown, unknown.runtime_id));
}

#[test]
fn type_search_uses_captured_type_and_query_admission_preserves_its_byte_budget() {
    let mut entity = target(CameraTargetKind::Entity);
    entity.name = Arc::from("Decorative subject");
    entity.entity_type = Some(Arc::from("minecraft:armor_stand"));
    let search = CameraSubjectSearch::new("ARMOR_").unwrap();
    assert!(search.matches(&entity));
    assert!(!search.includes(CameraSubjectFilter::Players, &entity, 0));
    assert!(search.includes(CameraSubjectFilter::All, &entity, 0));
    assert!(CameraSubjectSearch::new(" \t").is_err());
    let blank = CameraSubjectSearch::new("   ").unwrap();
    assert!(blank.is_empty() && blank.matches(&entity));
    assert_eq!(blank.text(), "   ");
    let budget = super::super::super::MAX_TARGET_NAME_BYTES;
    assert!(CameraSubjectSearch::new(&"a".repeat(budget)).is_ok());
    assert!(CameraSubjectSearch::new(&"a".repeat(budget + 1)).is_err());
}
