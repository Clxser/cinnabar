//! Changed replay cameras reorder complete recorded billboards while retaining their exact artwork.

use super::*;
use render_model::NametagScene;

impl ReplayRenderer {
    /// Billboard orientation comes from the active camera; only its back-to-front group order changes.
    pub(super) fn apply_labels(
        &mut self,
        frame: &SceneFrame,
        transform: Transform,
        source_view: bool,
    ) {
        let mut scene = frame
            .visuals
            .as_ref()
            .and_then(|visuals| visuals.nametags.as_deref())
            .cloned()
            .unwrap_or_default();
        if !source_view {
            order_labels(&mut scene, transform.translation, *transform.forward());
        }
        self.app
            .world_mut()
            .insert_resource(render_scene::NametagSceneResource(scene));
    }
}

/// Native labels publish contiguous groups sharing a head anchor and multiline lift. Plate/glyph,
/// through-wall/depth and cosmetic-border order inside each group must remain unchanged.
fn order_labels(scene: &mut NametagScene, position: Vec3, forward: Vec3) {
    let mut groups = Vec::new();
    let mut begin = 0;
    while begin < scene.records.len() {
        let first = &scene.records[begin];
        let mut end = begin + 1;
        while end < scene.records.len()
            && scene.records[end].anchor == first.anchor
            && scene.records[end].line_lift == first.line_lift
        {
            end += 1;
        }
        let anchor = Vec3::from_array(first.anchor) + Vec3::Y * first.line_lift;
        groups.push((forward.dot(anchor - position), begin..end));
        begin = end;
    }
    groups.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.start.cmp(&b.1.start)));
    let original = std::mem::take(&mut scene.records);
    scene.records = groups
        .into_iter()
        .flat_map(|(_, range)| original[range].iter().copied())
        .collect();
}

#[cfg(test)]
mod tests {
    use super::*;
    use render_model::{NametagAtlasRect, NametagRecord};

    #[test]
    fn camera_rotation_reorders_whole_recorded_labels_and_keeps_artwork_and_passes() {
        let mut scene = NametagScene::default();
        scene.atlas = Arc::from([NametagAtlasRect {
            cell: [0, 0, 1, 1],
            rgba8: Arc::from([255; 4]),
        }]);
        for z in [-4.0, -2.0] {
            for (text, depth_tested) in [(0, 0), (1, 0), (1, 1)] {
                scene.records.push(NametagRecord {
                    anchor: [0.0, 2.0, z],
                    text,
                    depth_tested,
                    ..Default::default()
                });
            }
        }
        let original = scene.records.clone();
        let atlas = scene.atlas.clone();
        order_labels(&mut scene, Vec3::ZERO, Vec3::Z);
        assert_eq!(&scene.records[..3], &original[3..]);
        assert_eq!(&scene.records[3..], &original[..3]);
        assert!(Arc::ptr_eq(&scene.atlas, &atlas));
        order_labels(&mut scene, Vec3::ZERO, Vec3::NEG_Z);
        assert_eq!(scene.records, original);
    }
}
