//! Bedrock 1.12 rules for an ordinary player attack.

use super::{legacy_cos, legacy_sin, partial_tick};
use glam::{Mat4, Vec3};
use render_api::BEDROCK_112;
use std::f32::consts::PI;

/// Attack deltas in authored model angles, with shoulder points in the mirrored rig frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LegacySwingPose {
    pub body_yaw: f32,
    pub right_rotation: Vec3,
    pub left_rotation: Vec3,
    pub right_shoulder: Vec3,
    pub left_shoulder: Vec3,
}

/// Samples the nonlinear strike before composing arm, sleeve and held-item descendants.
pub fn legacy_swing_pose(progress: f32, head_pitch: f32) -> LegacySwingPose {
    let rules = BEDROCK_112;
    let progress = partial_tick(progress);
    let body_yaw = rules.swing_body_radians * legacy_sin(progress.sqrt() * (PI * 2.0));
    let remaining = 1.0 - progress;
    let squared = remaining * remaining;
    let eased = legacy_sin(PI - (squared * squared) * PI);
    let pitch = -rules.swing_strike_radians * eased
        + rules.swing_head_gain
            * legacy_sin(progress * PI)
            * (head_pitch - rules.swing_head_offset);
    let shoulder = Vec3::new(
        rules.swing_shoulder_pixels * legacy_cos(body_yaw),
        0.0,
        rules.swing_shoulder_pixels * legacy_sin(body_yaw),
    );
    LegacySwingPose {
        body_yaw,
        right_rotation: Vec3::new(pitch, rules.swing_main_yaw_gain * body_yaw, 0.0),
        left_rotation: Vec3::new(body_yaw, body_yaw, 0.0),
        right_shoulder: shoulder,
        left_shoulder: -shoulder,
    }
}

/// The native empty-hand stack in camera space, before camera bob, hurt and sway.
/// Its geometry retains its rest pose in the client's mirrored rig frame, in blocks.
pub fn legacy_first_person_arm(progress: f32, equip: f32) -> Mat4 {
    let rules = BEDROCK_112;
    let progress = partial_tick(progress);
    let root = progress.sqrt() * PI;
    let swing = swing_translation(progress, rules.arm_swing_translation);
    let mut anchor = Vec3::from_array(rules.arm_anchor);
    anchor.y += rules.equip_translation * partial_tick(equip);
    let native = Mat4::from_translation(swing)
        * Mat4::from_translation(anchor)
        * Mat4::from_rotation_y(rules.arm_base_yaw)
        * Mat4::from_rotation_y(rules.arm_swing_yaw * legacy_sin(root))
        * Mat4::from_rotation_z(rules.arm_swing_roll * legacy_sin(progress * progress * PI))
        * Mat4::from_translation(Vec3::from_array(rules.arm_grip_translation))
        * Mat4::from_rotation_z(rules.arm_grip_radians[0])
        * Mat4::from_rotation_x(rules.arm_grip_radians[1])
        * Mat4::from_rotation_y(rules.arm_grip_radians[2])
        * Mat4::from_translation(Vec3::from_array(rules.arm_model_translation));
    let rig_from_model =
        Mat4::from_translation(Vec3::Y * assets::gui_item::SHIELD_MODEL_PART_HEIGHT / 16.0)
            * Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0));
    native * rig_from_model.inverse()
}

/// Main-hand camera placement with the existing fixed item display before the swing turns.
pub fn legacy_item_swing(progress: f32, equip: f32, display: Mat4) -> Mat4 {
    let rules = BEDROCK_112;
    let progress = partial_tick(progress);
    let root_sine = legacy_sin(progress.sqrt() * PI);
    let mut anchor = Vec3::from_array(rules.item_anchor);
    anchor.y += rules.equip_translation * partial_tick(equip);
    Mat4::from_translation(swing_translation(progress, rules.item_swing_translation))
        * Mat4::from_translation(anchor)
        * display
        * Mat4::from_rotation_y(rules.item_swing_radians[0] * legacy_sin(progress * progress * PI))
        * Mat4::from_rotation_z(rules.item_swing_radians[1] * root_sine)
        * Mat4::from_rotation_x(rules.item_swing_radians[2] * root_sine)
        * Mat4::from_scale(Vec3::splat(rules.item_scale))
}

/// Shared root-progress translation waves, with separate arm and item gains.
fn swing_translation(progress: f32, gain: [f32; 3]) -> Vec3 {
    let root = progress.sqrt() * PI;
    Vec3::new(
        legacy_sin(root),
        legacy_sin(root * 2.0),
        legacy_sin(progress * PI),
    ) * Vec3::from_array(gain)
}

#[cfg(test)]
mod tests;
