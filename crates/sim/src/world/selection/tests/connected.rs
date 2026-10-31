use super::*;
use assets::facts::connection::ConnectionFacts;
use assets::{ContributorRole, ModelFamily};

/// Supplies primary link facts for a focused neighbour test.
fn facts(family: ModelFamily) -> ConnectionFacts {
    ConnectionFacts {
        family,
        role: ContributorRole::Primary,
        occludes_full_face: false,
        links_as_unit_cube: false,
        orientation: None,
        nether_fence: false,
    }
}

#[test]
fn panes_pick_neighbour_links_and_keep_state_boxes_beside_unknown_blocks() {
    let post = ShapeBox::new([0.4375, 0.0, 0.4375], [0.5625, 1.0, 0.5625]);
    let target = shapes(SelectionContext::Static, vec![post], vec![]);
    for space in [CollisionIdSpace::Sequential, CollisionIdSpace::Hashed] {
        let (mut store, mut registry, id) = fixture(space, &target);
        assert!(registry.set_connection_facts(0, facts(ModelFamily::Air)));
        assert!(registry.set_connection_facts(id, facts(ModelFamily::Pane)));
        // Runtime 9 links as a pane; runtime 10 is registered without link facts.
        registry.register(9, [box_to_aabb(UNIT)]).unwrap();
        registry.set_connection_facts(9, facts(ModelFamily::Pane));
        registry.register(10, [box_to_aabb(UNIT)]).unwrap();
        let resolve = |store: &ChunkStore| {
            let view = PaletteWorld::new(store, &registry, 0, Default::default());
            view.selection_shapes_at([0; 3], registry.physics(id).unwrap())
                .unwrap()
                .into_owned()
        };
        assert_eq!(resolve(&store), [box_to_aabb(post)]);
        set_block(&mut store, [1, 0, 0], 9);
        set_block(&mut store, [0, 0, -1], 9);
        assert_eq!(
            resolve(&store),
            [box_to_aabb(ShapeBox::new(
                [0.4375, 0.0, 0.0],
                [1.0, 1.0, 0.5625]
            ))]
        );
        set_block(&mut store, [-1, 0, 0], 10);
        assert_eq!(resolve(&store), [box_to_aabb(post)]);
    }
}

#[test]
fn connected_picks_follow_neighbour_updates_instead_of_stored_links() {
    for family in [ModelFamily::Pane, ModelFamily::Fence] {
        // Deliberately store a full envelope: an isolated block must still pick only its post.
        let target = shapes(SelectionContext::Static, vec![UNIT], vec![]);
        for space in [CollisionIdSpace::Sequential, CollisionIdSpace::Hashed] {
            let (mut store, mut registry, id) = fixture(space, &target);
            registry.set_connection_facts(0, facts(ModelFamily::Air));
            registry.set_connection_facts(id, facts(family));
            let origin = Vec3::new(0.875, 0.5, -0.5);
            let direction = Vec3::new(0.0, 0.0, 1.0);
            for linked in [false, true, false] {
                set_block(&mut store, [1, 0, 0], if linked { id } else { 0 });
                let view = PaletteWorld::new(&store, &registry, 0, Default::default());
                let hit = view
                    .block_interaction_ray_current(origin, direction, 1.5)
                    .unwrap();
                assert_eq!(hit.is_some(), linked, "{family:?} {space:?}");
                let aim = view
                    .camera_aim_ray(origin, origin + direction * 1.5, 8, true, false)
                    .unwrap();
                assert_eq!(aim.is_some(), linked, "{family:?} {space:?}");
                if let Some(hit) = hit {
                    let inset = if family == ModelFamily::Pane {
                        0.4375
                    } else {
                        0.375
                    };
                    assert_eq!(hit.block_pos, [0; 3]);
                    assert_eq!(hit.face, 2);
                    assert_eq!(hit.hit_local, Vec3::new(0.875, 0.5, inset));
                    assert_eq!(hit.distance, 0.5 + inset);
                    let aim = aim.unwrap();
                    assert_eq!(aim.block_pos, hit.block_pos);
                    assert_eq!(aim.face, hit.face);
                    assert_eq!(aim.hit_local, hit.hit_local);
                }
            }
        }
    }
}

#[test]
fn connected_pick_identity_includes_the_neighbour_column() {
    let target = shapes(SelectionContext::Static, vec![UNIT], vec![]);
    for space in [CollisionIdSpace::Sequential, CollisionIdSpace::Hashed] {
        let (mut store, mut registry, id) = fixture(space, &target);
        registry.set_connection_facts(0, facts(ModelFamily::Air));
        registry.set_connection_facts(id, facts(ModelFamily::Pane));
        set_block(&mut store, [0; 3], 0);
        set_block(&mut store, [15, 0, 0], id);
        set_block(&mut store, [16, 0, 0], id);
        let origin = Vec3::new(15.875, 0.5, -0.5);
        let direction = Vec3::new(0.0, 0.0, 1.0);
        let hit = PaletteWorld::new(&store, &registry, 0, Default::default())
            .block_interaction_ray_current(origin, direction, 1.5)
            .unwrap()
            .unwrap();
        assert_eq!(hit.block_pos, [15, 0, 0]);
        set_block(&mut store, [16, 0, 0], 0);
        let view = PaletteWorld::new(&store, &registry, 0, Default::default());
        assert!(matches!(
            view.block_interaction_ray(origin, direction, 1.5, &hit.identity),
            Err(crate::WorldQueryError::StaleCollisionIdentity { chunk })
                if chunk == world::ChunkKey::new(0, 1, 0)
        ));
        assert_eq!(
            view.block_interaction_ray_current(origin, direction, 1.5)
                .unwrap(),
            None
        );
    }
}
