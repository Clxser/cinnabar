use super::*;

fn targets(slots: &[u8]) -> Vec<InventoryTarget> {
    slots.iter().copied().map(InventoryTarget::Player).collect()
}

/// Each added slot plans only transfers and updates their cells, independent of session size.
#[test]
fn drag_steps_do_not_clone_the_ledger_or_refold_pending_history() {
    for mode in [DistributeMode::Even, DistributeMode::One] {
        for craft in [false, true] {
            let mut ledger = personal_ledger(&[]);
            set_cursor(&mut ledger, stack(60, 10));
            let mut drag = None;
            let mut targets = Vec::new();
            for index in 0..4 {
                targets.push(if craft {
                    InventoryTarget::Craft(*protocol::CRAFTING_INPUT_SLOTS.start() + index)
                } else {
                    InventoryTarget::Player(9 + index)
                });
                if targets.len() < 2 {
                    continue;
                }
                let before = ledger.queue.len();
                super::super::distribute::work::take();
                ledger
                    .advance_distribute(&mut drag, &targets, mode)
                    .unwrap();
                assert_eq!(super::super::distribute::work::take(), (0, 0));
                let transfers = match (mode, index) {
                    (_, 1) => 2,
                    (DistributeMode::Even, _) => 3,
                    (DistributeMode::One, _) => 1,
                };
                assert_eq!(ledger.queue.len() - before, transfers);
                let incremental: Vec<_> = ledger
                    .view()
                    .occupied()
                    .map(|(cell, held)| (cell, held.clone()))
                    .collect();
                ledger.refold();
                let folded: Vec<_> = ledger
                    .view()
                    .occupied()
                    .map(|(cell, held)| (cell, held.clone()))
                    .collect();
                assert_eq!(
                    incremental, folded,
                    "sparse identities and item lineage match"
                );
            }
        }
    }
}

/// Rebalancing sends separate transfers, including a remainder returned to the cursor.
#[test]
fn incremental_drag_preserves_request_scopes_and_wire_actions() {
    let mut ledger = personal_ledger(&[]);
    set_cursor(&mut ledger, stack(60, 10));
    let mut drag = None;
    for slots in [&[9, 10][..], &[9, 10, 11][..]] {
        ledger
            .advance_distribute(&mut drag, &targets(slots), DistributeMode::Even)
            .unwrap();
    }
    let slot = |container, slot, stack_network_id| protocol::StackRequestSlot {
        container,
        slot,
        stack_network_id,
    };
    let player = StackRequestContainer::PlayerInventory;
    let cursor = StackRequestContainer::Cursor;
    let expected = [
        StackRequestAction::Place {
            amount: 5,
            source: slot(cursor, 0, 60),
            destination: slot(player, 9, 0),
        },
        StackRequestAction::Place {
            amount: 5,
            source: slot(cursor, 0, -3),
            destination: slot(player, 10, 0),
        },
        StackRequestAction::Place {
            amount: 2,
            source: slot(player, 9, -3),
            destination: slot(player, 11, 0),
        },
        StackRequestAction::Place {
            amount: 1,
            source: slot(player, 10, -5),
            destination: slot(player, 11, -7),
        },
        StackRequestAction::Take {
            amount: 1,
            source: slot(player, 10, -9),
            destination: slot(cursor, 0, -5),
        },
    ];
    assert_eq!(ledger.queue.len(), expected.len());
    for (index, (request, action)) in ledger.queue.iter().zip(expected).enumerate() {
        assert_eq!(request.request_id, -3 - 2 * index as i32);
        assert_eq!(request.actions, [action]);
        assert!(protocol::item_stack_request_packet(request.request_id, &request.actions).is_ok());
    }
}

/// Exhausting the ID range during planning cannot publish the earlier transfer.
#[test]
fn drag_request_id_exhaustion_is_atomic() {
    let mut ledger = personal_ledger(&[]);
    set_cursor(&mut ledger, stack(60, 10));
    ledger.next_request_id = i32::MIN + 3;
    let mut drag = None;
    assert_eq!(
        ledger.advance_distribute(&mut drag, &targets(&[9, 10]), DistributeMode::Even),
        Err(InventoryGestureError::InvalidRequest)
    );
    assert_eq!(ledger.next_request_id, i32::MIN + 3);
    assert!(ledger.queue.is_empty());
    assert!(drag.is_none());
    assert_eq!(ledger.cursor_stack().unwrap().count, 10);
    assert!(ledger.displayed_stack(9).is_none());
}

#[test]
fn live_even_split_rebalances_before_any_response_with_an_empty_cursor() {
    let mut ledger = personal_ledger(&[]);
    set_cursor(&mut ledger, stack(60, 10));
    let mut drag = None;
    let first = ledger
        .advance_distribute(&mut drag, &targets(&[9, 10]), DistributeMode::Even)
        .unwrap()
        .unwrap();
    assert!(ledger.cursor_stack().is_none());
    assert_eq!(ledger.displayed_stack(9).unwrap().count, 5);
    assert_eq!(ledger.displayed_stack(10).unwrap().count, 5);
    let second = ledger
        .advance_distribute(&mut drag, &targets(&[9, 10, 11]), DistributeMode::Even)
        .unwrap()
        .unwrap();
    assert_ne!(first, second);
    for slot in [9, 10, 11] {
        assert_eq!(ledger.displayed_stack(slot).unwrap().count, 3);
    }
    assert_eq!(ledger.cursor_stack().unwrap().count, 1);
    assert!(
        ledger
            .queue
            .iter()
            .filter(|request| request.request_id < first)
            .flat_map(|request| &request.actions)
            .any(|action| matches!(
                action, StackRequestAction::Place { source, .. } if source.stack_network_id == first
            ))
    );
    assert_eq!(
        ledger.advance_distribute(&mut drag, &targets(&[9, 10, 11]), DistributeMode::Even),
        Ok(None)
    );
}

#[test]
fn live_split_only_rebalances_its_own_items_in_preexisting_stacks() {
    let mut ledger = personal_ledger(&[(9, stack(11, 50))]);
    set_cursor(&mut ledger, stack(60, 10));
    let mut drag = None;
    ledger
        .advance_distribute(&mut drag, &targets(&[9, 10]), DistributeMode::Even)
        .unwrap();
    assert_eq!(ledger.displayed_stack(9).unwrap().count, 55);
    ledger
        .advance_distribute(&mut drag, &targets(&[9, 10, 11]), DistributeMode::Even)
        .unwrap();
    assert_eq!(ledger.displayed_stack(9).unwrap().count, 53);
    assert_eq!(ledger.displayed_stack(10).unwrap().count, 3);
    assert_eq!(ledger.displayed_stack(11).unwrap().count, 3);
    assert_eq!(ledger.cursor_stack().unwrap().count, 1);
}

#[test]
fn live_secondary_split_does_not_repeat_a_visited_slot() {
    let mut ledger = personal_ledger(&[]);
    set_cursor(&mut ledger, stack(60, 5));
    let mut drag = None;
    ledger
        .advance_distribute(&mut drag, &targets(&[9, 10]), DistributeMode::One)
        .unwrap();
    ledger
        .advance_distribute(&mut drag, &targets(&[9, 10, 9, 11]), DistributeMode::One)
        .unwrap();
    for slot in [9, 10, 11] {
        assert_eq!(ledger.displayed_stack(slot).unwrap().count, 1);
    }
    assert_eq!(ledger.cursor_stack().unwrap().count, 2);
    assert_eq!(ledger.queue.len(), 3);
    for (index, request) in ledger.queue.iter().enumerate() {
        let request_id = -3 - 2 * index as i32;
        assert_eq!(request.request_id, request_id);
        assert_eq!(
            request.actions,
            [StackRequestAction::Place {
                amount: 1,
                source: protocol::StackRequestSlot {
                    container: StackRequestContainer::Cursor,
                    slot: 0,
                    stack_network_id: if index == 0 { 60 } else { request_id + 2 },
                },
                destination: protocol::StackRequestSlot {
                    container: StackRequestContainer::PlayerInventory,
                    slot: 9 + index as u8,
                    stack_network_id: 0,
                },
            }]
        );
    }
}

#[test]
fn live_split_caps_merges_and_skips_an_incompatible_slot() {
    let mut incompatible = stack(21, 3);
    incompatible.metadata = 1;
    let mut ledger = personal_ledger(&[(9, stack(11, 63)), (10, incompatible.clone())]);
    set_cursor(&mut ledger, stack(60, 10));
    let mut drag = None;
    ledger
        .advance_distribute(&mut drag, &targets(&[9, 10, 11]), DistributeMode::Even)
        .unwrap();
    assert_eq!(ledger.displayed_stack(9).unwrap().count, 64);
    assert_eq!(ledger.displayed_stack(10), Some(&incompatible));
    assert_eq!(ledger.displayed_stack(11).unwrap().count, 5);
    assert_eq!(ledger.cursor_stack().unwrap().count, 4);
    ledger
        .advance_distribute(&mut drag, &targets(&[9, 10, 11, 12]), DistributeMode::Even)
        .unwrap();
    assert_eq!(ledger.displayed_stack(9).unwrap().count, 64);
    assert_eq!(ledger.displayed_stack(11).unwrap().count, 3);
    assert_eq!(ledger.displayed_stack(12).unwrap().count, 3);
    assert_eq!(ledger.cursor_stack().unwrap().count, 3);
}

#[test]
fn another_gesture_invalidates_split_accounting_instead_of_stealing_items() {
    let mut ledger = personal_ledger(&[]);
    set_cursor(&mut ledger, stack(60, 10));
    let mut drag = None;
    ledger
        .advance_distribute(&mut drag, &targets(&[9, 10]), DistributeMode::Even)
        .unwrap();
    ledger.begin_click(9).unwrap();
    let before = ledger.newest_request().unwrap().request_id;
    assert!(
        ledger
            .advance_distribute(&mut drag, &targets(&[9, 10, 11]), DistributeMode::Even)
            .is_err()
    );
    assert_eq!(ledger.newest_request().unwrap().request_id, before);
    assert!(ledger.displayed_stack(11).is_none());
}

#[test]
fn queue_limit_does_not_partially_submit_a_hover_rebalance() {
    let mut ledger = personal_ledger(&[]);
    set_cursor(&mut ledger, stack(60, 10));
    let mut drag = None;
    ledger
        .advance_distribute(&mut drag, &targets(&[9, 10]), DistributeMode::Even)
        .unwrap();
    let mut inactive = ledger.queue[0].clone();
    for prediction in &mut inactive.predicted {
        prediction.active = false;
    }
    while ledger.queue.len() < MAX_PENDING_REQUESTS - 1 {
        ledger.queue.push_back(inactive.clone());
    }
    let next_id = ledger.next_request_id;
    assert_eq!(
        ledger.advance_distribute(&mut drag, &targets(&[9, 10, 11]), DistributeMode::Even),
        Err(InventoryGestureError::Busy)
    );
    assert_eq!(ledger.queue.len(), MAX_PENDING_REQUESTS - 1);
    assert_eq!(ledger.next_request_id, next_id);
    assert_eq!(ledger.displayed_stack(9).unwrap().count, 5);
    assert_eq!(ledger.displayed_stack(10).unwrap().count, 5);
    assert!(ledger.displayed_stack(11).is_none());
    assert!(ledger.cursor_stack().is_none());
}

/// A press-time place-all becomes the first share once the drag reaches a second cell.
#[test]
fn press_time_placement_rebalances_when_the_drag_reaches_a_second_cell() {
    let mut ledger = personal_ledger(&[]);
    set_cursor(&mut ledger, stack(60, 10));
    let template = ledger.cursor_stack().unwrap().clone();
    ledger
        .begin_target_gesture(InventoryTarget::Player(9), CellGesture::Click)
        .unwrap();
    assert!(ledger.cursor_stack().is_none());
    let mut drag = ledger.distribution_after_place(
        &template,
        InventoryTarget::Player(9),
        None,
        DistributeMode::Even,
    );
    assert!(drag.is_some());
    ledger
        .advance_distribute(&mut drag, &targets(&[9, 10]), DistributeMode::Even)
        .unwrap();
    assert_eq!(ledger.displayed_stack(9).unwrap().count, 5);
    assert_eq!(ledger.displayed_stack(10).unwrap().count, 5);
    assert!(ledger.cursor_stack().is_none());
}
