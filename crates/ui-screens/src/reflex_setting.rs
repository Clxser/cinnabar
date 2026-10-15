//! NVIDIA Reflex selector in the JSON-UI Video settings.

use json_ui::Catalog;
use serde_json::json;

use launcher::menu::settings_options::{REFLEX_CHOICES, REFLEX_OPTION};

/// Adds the persisted Reflex choice to the advanced Video controls.
pub fn install(catalog: &mut Catalog) {
    let name = REFLEX_OPTION.name;
    let choices = REFLEX_CHOICES
        .iter()
        .map(|choice| {
            json!({
                "@settings_common.radio_with_label": {
                    "$toggle_state_binding_name": format!("#{}", choice.name),
                    "$radio_label_text": choice.label
                }
            })
        })
        .collect::<Vec<_>>();
    let overlay = json!({
        "namespace": "general_section",
        format!("{name}_dropdown_content@settings_common.option_radio_dropdown_group"): {
            "$radio_buttons": choices
        },
        "advanced_graphics_options_section": {
            "modifications": [{
                "array_name": "controls",
                "operation": "insert_after",
                "control_name": "vrr",
                "value": [{
                    format!("{name}@settings_common.option_dropdown"): {
                        "$option_label": REFLEX_OPTION.label,
                        "$dropdown_content": format!("general_section.{name}_dropdown_content"),
                        "$dropdown_area": "content_area",
                        "$dropdown_name": format!("{name}_dropdown"),
                        "$option_enabled_binding_name": format!("#{name}_dropdown_enabled"),
                        "$options_dropdown_toggle_label_binding": format!("#{name}_dropdown_toggle_label")
                    }
                }]
            }]
        }
    });
    catalog.overlay_text("ui/cinnabar_reflex.json", &overlay.to_string());
}
