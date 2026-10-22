//! Client-authoritative breaks report a worn tool through their transaction.

use super::*;

/// A client-authoritative break that wears the held tool reports the worn stack as the
/// transaction's slot change under a fresh legacy request; an unworn or breaking tool follows
/// the same rule as the stack it leaves.
#[test]
fn client_destroys_carry_the_worn_tool_as_their_slot_change() {
    let sword = VerifiedNetworkItemStack::try_new(
        NetworkItemStack {
            network_id: 300,
            count: 1,
            stack_network_id: 12,
            ..NetworkItemStack::empty()
        },
        NetworkItemStack::empty().nbt_digest,
    )
    .unwrap();
    let complete = |current_damage, break_damage, maximum| {
        let mut target = DestroyTarget {
            wear: Some(ToolWear {
                current_damage,
                break_damage,
                maximum,
            }),
            ..target([0, 0, 0], "minecraft:dirt", Some("minecraft:iron_sword"))
        };
        target.selection.item = sword.clone();
        let mut machine = DestroyMachine::default();
        held(&mut machine, &target, Client);
        let done = loop {
            let payload = held(&mut machine, &target, Client);
            if payload.destroy.is_some() {
                break payload;
            }
        };
        let (interactions, request) = done.into_interactions([0.5, 64.0, 0.5]);
        assert_eq!(request, None, "client authority sends no mining request");
        match interactions.block_interaction {
            Some(protocol::BlockItemInteraction::Destroy(_, change)) => change,
            other => panic!("expected a destroy transaction, got {other:?}"),
        }
    };
    let change = complete(4, 2, Some(250)).expect("a worn tool");
    assert_eq!(change.from, sword);
    assert!(protocol::is_legacy_request_id(change.legacy_request_id));
    assert_eq!(change.to.stack_network_id(), change.legacy_request_id);
    assert_eq!(change.to.network_id(), 300);
    assert_eq!(change.to.count(), 1);
    assert_eq!(protocol::item_extra_damage(change.to.extra_data()), Some(6));
    assert_eq!(
        complete(4, 0, Some(250)),
        None,
        "a spared tool is unchanged"
    );
    let broken = complete(249, 2, Some(250)).expect("a breaking tool");
    assert!(
        broken.to.is_empty(),
        "damage past the maximum breaks the tool"
    );
}
