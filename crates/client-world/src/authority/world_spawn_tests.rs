use super::*;

#[test]
fn world_spawn_starts_at_the_bootstrap_spawn_and_follows_world_spawn_updates() {
    let mut authority = WorldAuthority::new(
        WorldBootstrap {
            local_player_unique_id: 1,
            local_player_runtime_id: 1,
            dimension: 0,
            player_position: [0.0; 3],
            world_spawn_position: [12, 64, -30],
            air_network_id: 0,
            block_network_ids_are_hashes: false,
        },
        Arc::new(RuntimeAssets::diagnostic()),
        None,
        [0.0; 3],
        None,
    );
    assert_eq!(authority.world_spawn(), [12, 64, -30]);
    authority
        .apply_ordered_event(
            WorldEvent::WorldSpawn(protocol::WorldSpawnEvent {
                position: [-5, 70, 8],
            }),
            Some(1),
        )
        .unwrap();
    assert_eq!(authority.world_spawn(), [-5, 70, 8]);
}
