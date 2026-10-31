use super::*;

/// Uses a disconnected palette state so only the surrounding world supplies links.
fn disconnected(name: &str) -> &'static RegistryRecord {
    fixture()
        .records
        .iter()
        .find(|record| {
            record.name.as_ref() == name && {
                let state: serde_json::Value =
                    serde_json::from_str(&record.canonical_state).unwrap();
                ["west", "east", "north", "south"].into_iter().all(|side| {
                    state[format!("minecraft:connection_{side}")]["value"].as_u64() == Some(0)
                })
            }
        })
        .unwrap()
}

#[test]
fn connected_panes_and_fences_pick_world_connections_in_both_id_spaces() {
    let names: std::collections::BTreeSet<_> = fixture()
        .records
        .iter()
        .filter(|record| {
            matches!(
                record.model_family,
                assets::ModelFamily::Pane | assets::ModelFamily::Fence
            )
        })
        .map(|record| record.name.as_ref())
        .collect();
    assert!(names.contains("minecraft:green_stained_glass_pane"));
    assert!(names.contains("minecraft:iron_bars"));
    assert!(names.contains("minecraft:spruce_fence"));
    for name in names {
        let record = disconnected(name);
        let inset = if record.model_family == assets::ModelFamily::Pane {
            0.4375
        } else {
            0.375
        };
        for mode in [NetworkIdMode::Sequential, NetworkIdMode::Hashed] {
            let id = runtime_id(record, mode);
            for mask in 0..16 {
                let mut store = store(mode, record);
                for (bit, pos) in [[7, 8, 8], [9, 8, 8], [8, 8, 7], [8, 8, 9]]
                    .into_iter()
                    .enumerate()
                {
                    if mask & (1 << bit) != 0 {
                        put(&mut store, pos, id, mode);
                    }
                }
                let world = PaletteWorld::new(
                    &store,
                    fixture().registries.registry(mode),
                    0,
                    Default::default(),
                );
                let min = [
                    if mask & 1 != 0 { 0.0 } else { inset },
                    0.0,
                    if mask & 4 != 0 { 0.0 } else { inset },
                ];
                let max = [
                    if mask & 2 != 0 { 1.0 } else { 1.0 - inset },
                    1.0,
                    if mask & 8 != 0 { 1.0 } else { 1.0 - inset },
                ];
                let expected = bounds(min, max).translated(Vec3::new(8.0, 8.0, 8.0));
                assert_eq!(
                    world.block_outline([8; 3], id).unwrap(),
                    Some(expected),
                    "{name} {mode:?} mask={mask}"
                );
                // Aim through each arm and through the corner of the native bounding envelope.
                for x in [0.125, 0.5, 0.875] {
                    for z in [0.125, 0.5, 0.875] {
                        let origin = Vec3::new(8.0 + x, 9.5, 8.0 + z);
                        let hit = world
                            .block_interaction_ray_current(origin, Vec3::new(0.0, -1.0, 0.0), 1.0)
                            .unwrap();
                        let inside = x >= f64::from(min[0])
                            && x <= f64::from(max[0])
                            && z >= f64::from(min[2])
                            && z <= f64::from(max[2]);
                        assert_eq!(hit.is_some(), inside, "{name} mask={mask} x={x} z={z}");
                        if let Some(hit) = hit {
                            assert_eq!(hit.block_pos, [8; 3]);
                            assert_eq!(hit.face, 1);
                            assert_eq!(hit.hit_local, Vec3::new(x, 1.0, z));
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn wall_connections_follow_palette_states() {
    for record in fixture()
        .records
        .iter()
        .filter(|record| record.model_family == assets::ModelFamily::Wall)
    {
        let connections = record
            .model_state
            .get(ModelStateField::Connections)
            .unwrap();
        let [north, east, south, west] =
            std::array::from_fn::<_, 4, _>(|side| connections >> (2 * side) & 3);
        let post = connections & (1 << 8) != 0;
        let height = if post || [north, east, south, west].contains(&2) {
            0.9999_f32
        } else {
            0.8125
        };
        let thin_x = !post && north != 0 && south != 0 && east == 0 && west == 0;
        let thin_z = !post && east != 0 && west != 0 && north == 0 && south == 0;
        let min = [
            if west != 0 {
                0.0
            } else if thin_x {
                0.3125
            } else {
                0.25
            },
            0.0,
            if north != 0 {
                0.0
            } else if thin_z {
                0.3125
            } else {
                0.25
            },
        ];
        let max = [
            if east != 0 {
                1.0
            } else if thin_x {
                0.6875
            } else {
                0.75
            },
            height,
            if south != 0 {
                1.0
            } else if thin_z {
                0.6875
            } else {
                0.75
            },
        ];
        // Border blocks have wall states but retain a unit visual box.
        let (min, max) = if record.name.as_ref() == "minecraft:border_block" {
            ([0.0; 3], [1.0; 3])
        } else {
            (min, max)
        };
        for mode in [NetworkIdMode::Sequential, NetworkIdMode::Hashed] {
            let id = runtime_id(record, mode);
            let store = store(mode, record);
            let world = PaletteWorld::new(
                &store,
                fixture().registries.registry(mode),
                0,
                Default::default(),
            );
            assert_eq!(
                world.block_outline([8; 3], id).unwrap(),
                Some(bounds(min, max).translated(Vec3::new(8.0, 8.0, 8.0))),
                "{} {} {mode:?}",
                record.name,
                record.canonical_state
            );
            let hit = world
                .block_interaction_ray_current(
                    Vec3::new(8.5, 9.25, 8.5),
                    Vec3::new(0.0, -1.0, 0.0),
                    1.0,
                )
                .unwrap()
                .unwrap();
            assert_eq!(hit.block_pos, [8; 3]);
            assert_eq!(hit.face, 1);
            assert_eq!(hit.hit_local.y, f64::from(8.0_f32 + max[1]) - 8.0);
        }
    }
}

#[test]
fn connected_selection_uses_neighbor_material_and_gate_axis() {
    for mode in [NetworkIdMode::Sequential, NetworkIdMode::Hashed] {
        for (center, neighbor, connects) in [
            (
                "minecraft:green_stained_glass_pane",
                "minecraft:iron_bars",
                true,
            ),
            (
                "minecraft:green_stained_glass_pane",
                "minecraft:cobblestone_wall",
                true,
            ),
            (
                "minecraft:green_stained_glass_pane",
                "minecraft:glass",
                true,
            ),
            (
                "minecraft:green_stained_glass_pane",
                "minecraft:stone",
                true,
            ),
            (
                "minecraft:green_stained_glass_pane",
                "minecraft:spruce_fence",
                false,
            ),
            ("minecraft:spruce_fence", "minecraft:oak_fence", true),
            (
                "minecraft:spruce_fence",
                "minecraft:nether_brick_fence",
                false,
            ),
            (
                "minecraft:nether_brick_fence",
                "minecraft:nether_brick_fence",
                true,
            ),
            ("minecraft:spruce_fence", "minecraft:poppy", false),
        ] {
            let target = disconnected(center);
            let id = runtime_id(target, mode);
            let mut store = store(mode, target);
            put(
                &mut store,
                [7, 8, 8],
                runtime_id(record(neighbor), mode),
                mode,
            );
            let world = PaletteWorld::new(
                &store,
                fixture().registries.registry(mode),
                0,
                Default::default(),
            );
            assert_eq!(
                world.block_outline([8; 3], id).unwrap().unwrap().min.x == 8.0,
                connects,
                "{center} -> {neighbor}"
            );
        }
        for gate in fixture()
            .records
            .iter()
            .filter(|record| record.model_family == assets::ModelFamily::Gate)
        {
            let target = disconnected("minecraft:spruce_fence");
            let id = runtime_id(target, mode);
            let mut store = store(mode, target);
            put(&mut store, [7, 8, 8], runtime_id(gate, mode), mode);
            let world = PaletteWorld::new(
                &store,
                fixture().registries.registry(mode),
                0,
                Default::default(),
            );
            assert_eq!(
                world.block_outline([8; 3], id).unwrap().unwrap().min.x == 8.0,
                gate.model_state.get(ModelStateField::Orientation).unwrap() & 1 == 0
            );
        }
    }
}
