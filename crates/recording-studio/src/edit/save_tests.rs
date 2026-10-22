use super::*;

#[test]
fn saving_an_older_inflight_snapshot_preserves_dirty_edits_and_undo_recognizes_the_saved_baseline()
{
    let mut editor = Editor::new(Project::new("initial")).unwrap();
    editor
        .apply(
            "first edit",
            &[Command::Rename {
                name: "saved snapshot".into(),
            }],
        )
        .unwrap();
    let saved = Arc::clone(editor.project());
    editor
        .apply(
            "later edit",
            &[Command::Rename {
                name: "unsaved later edit".into(),
            }],
        )
        .unwrap();
    editor.mark_saved_snapshot(saved);
    assert!(editor.dirty());
    editor.undo().unwrap();
    assert!(!editor.dirty());
    editor.redo().unwrap();
    assert!(editor.dirty());
}

#[test]
fn older_export_settings_receive_current_safe_defaults_without_dropping_known_values() {
    let settings: ExportSettings = serde_json::from_str(
        r#"{"size":[1280,720],"fps":{"numerator":60,"denominator":1},"codec":"h264"}"#,
    )
    .unwrap();
    assert_eq!(settings.size, [1280, 720]);
    assert_eq!(settings.encoder, crate::model::Encoder::Auto);
    assert!(!settings.include_recorded_hud);
    assert_eq!(settings.quality, ExportSettings::default().quality);
}
