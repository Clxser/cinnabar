//! Server WIT 0.12's content rules in callbacks: no stack of a block without an item, and
//! whether a break harvests.

use super::*;
use crate::protocol::{Cause, Change};

/// A block without an item makes no stack: neither a slot nor a drop takes one, and nothing is
/// staged.
#[test]
fn a_block_without_an_item_makes_no_stack() {
    let mut res = Fixture::new().res();
    let hidden = new_stack(HIDDEN, 1, None);
    assert_eq!(
        res.set_slot(0, Some(hidden.clone())).unwrap(),
        Err(WorldError::UnknownBlock)
    );
    assert_eq!(
        res.drop_item(UP, hidden).unwrap(),
        Err(WorldError::UnknownBlock)
    );
    assert!(res.ops.is_empty());
    // A block with an item still makes stacks.
    assert_eq!(
        res.drop_item(UP, new_stack(COUNTER, 1, None)).unwrap(),
        Ok(())
    );
}

/// `harvested` holds only in a break that the adapter marks as a harvest.
#[test]
fn only_a_harvesting_break_is_harvested() {
    assert!(!Fixture::new().res().harvested().unwrap());
    for harvest in [false, true] {
        let request = Request::Callback {
            seq: 1,
            info: Info {
                context: None,
                simulation: None,
                world_id: "world".to_owned(),
                dimension_id: "overworld".to_owned(),
                tick: 1,
                event_sequence: 1,
            },
            actor: Some(ACTOR.to_owned()),
            world_min_y: -64,
            world_max_y: 319,
            data_budget: 0,
            snapshot: vec![loaded(ANCHOR, AIR, false, None)],
            network: None,
            inventory: None,
            call: Call::Break {
                change: Change {
                    pos: ANCHOR,
                    actor: Some(ACTOR.to_owned()),
                    cause: Cause::Player,
                    before_id: COUNTER.to_owned(),
                    after_id: AIR.to_owned(),
                    previous_data: None,
                },
                harvest,
            },
        };
        let own = Catalog {
            blocks: [(COUNTER.to_owned(), Vec::new())].into(),
            ..Catalog::default()
        };
        let (mut res, _) = prepare(&Arc::new(own), &request).unwrap();
        assert_eq!(res.harvested().unwrap(), harvest);
    }
}
