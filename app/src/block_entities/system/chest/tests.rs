use super::*;
use crate::block_entities::system::columns::routed_entities;
use world::{BlockEntityNbt, DecodedBlockEntities, RawBlockIds, SubChunk};

/// Writes the network NBT unsigned varint encoding.
fn varint(mut value: u32, out: &mut Vec<u8>) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        out.push(byte | if value == 0 { 0 } else { 0x80 });
        if value == 0 {
            return;
        }
    }
}

/// Writes a network NBT string.
fn string(value: &str, out: &mut Vec<u8>) {
    varint(value.len() as u32, out);
    out.extend_from_slice(value.as_bytes());
}

/// What a chest's record says about its pair.
#[derive(Clone, Copy)]
enum Record {
    /// No pairing tags.
    Plain,
    /// `pairx`/`pairz` naming the partner, with `pairlead` when given.
    Pair([i32; 3], Option<bool>),
}

/// Encodes one chest record.
fn nbt(key: BlockEntityKey, record: Record) -> BlockEntityNbt {
    let mut out = vec![10, 0, 8];
    string("id", &mut out);
    string("Chest", &mut out);
    let (pair, lead) = match record {
        Record::Plain => (None, None),
        Record::Pair(at, lead) => (Some(at), lead),
    };
    for (name, value) in ["x", "y", "z"].into_iter().zip(key.position()).chain(
        pair.into_iter()
            .flat_map(|at| [("pairx", at[0]), ("pairz", at[2])]),
    ) {
        out.push(3);
        string(name, &mut out);
        varint(((value << 1) ^ (value >> 31)) as u32, &mut out);
    }
    if let Some(lead) = lead {
        out.push(1);
        string("pairlead", &mut out);
        out.push(u8::from(lead));
    }
    out.push(0);
    BlockEntityNbt::decode_prefix(&out).unwrap().0
}

/// Loaded blocks and actors described through the production registry and template path.
struct Fixture {
    store: ChunkStore,
    runtime: BlockEntityRuntime,
    collisions: PhysicsCollisionRegistries,
    records: Box<[assets::RegistryRecord]>,
    scans: HashMap<ChunkKey, ColumnScan>,
}

impl Fixture {
    /// Uses embedded registry fixtures and no external carriers.
    fn new() -> Self {
        let breg = assets::pinned_block_registry_bytes();
        let protocol = assets::active_content_registry_protocol();
        let records = assets::read_registry_for_protocol(breg, protocol).unwrap();
        let collisions = PhysicsCollisionRegistries::from_assets(
            breg,
            &records,
            include_bytes!("../../../../../crates/assets/data/block-physics-v2193.bin"),
            protocol,
        )
        .unwrap();
        Self {
            store: ChunkStore::new(),
            runtime: BlockEntityRuntime::new(),
            collisions,
            records,
            scans: HashMap::new(),
        }
    }

    /// Loads a section holding the named chest block with the given cardinal facing.
    fn block(&mut self, key: BlockEntityKey, name: &str, facing: &str) {
        let id = self.block_id(name, Some(facing));
        let mut out = vec![9, 1, key.sub_chunk().y as u8, 1];
        varint(id << 1, &mut out);
        self.store
            .commit_sub_chunk(
                key.sub_chunk(),
                SubChunk::decode(&out, &RawBlockIds { air: 0 }),
            )
            .unwrap();
    }

    /// Finds the registry ID used by one block update.
    fn block_id(&self, name: &str, facing: Option<&str>) -> u32 {
        self.records
            .iter()
            .find(|record| {
                record.name.as_ref() == name
                    && facing.is_none_or(|facing| {
                        record
                            .canonical_state
                            .contains(&format!("\"value\":\"{facing}\""))
                    })
            })
            .unwrap()
            .sequential_id
    }

    /// Applies the single-cell terrain update used when placing or clearing a chest.
    fn update_block(&mut self, key: BlockEntityKey, chest: bool) {
        let air = self.block_id("minecraft:air", None);
        let id = if chest {
            self.block_id("minecraft:chest", Some("north"))
        } else {
            air
        };
        self.store
            .update_block(
                key.sub_chunk(),
                world::BlockUpdate {
                    x: (key.x & 15) as u8,
                    y: (key.y & 15) as u8,
                    z: (key.z & 15) as u8,
                    layer: 0,
                    runtime_id: id,
                },
                air,
            )
            .unwrap();
    }

    /// Delivers a record in its level chunk's block-entity tail or as a later
    /// BlockActorData update.
    fn actor(&mut self, key: BlockEntityKey, record: Record, in_chunk: bool) {
        let nbt = nbt(key, record);
        if in_chunk {
            let mut tail = vec![0];
            tail.extend_from_slice(nbt.bytes());
            self.store
                .commit_chunk_block_entities(
                    key.chunk(),
                    DecodedBlockEntities::decode_level_chunk_tail(key.chunk(), 0..256, &tail),
                )
                .unwrap();
        } else {
            self.store.commit_block_entity_update(key, nbt).unwrap();
        }
    }

    /// The template the scene caches for `key` from its current block and record.
    fn template(&mut self, key: BlockEntityKey) -> ChestModel {
        let template = stored_template(
            key,
            &self.store,
            &mut self.runtime,
            &self.collisions,
            assets::NetworkIdMode::Sequential,
        );
        let Some(Template::Chest(model)) = template else {
            panic!("a chest template at {key:?}");
        };
        model
    }

    /// Scans `chunk` the way the scene does, so neighbours resolve from its templates.
    fn scan(&mut self, chunk: ChunkKey) {
        let column = self.store.chunk(chunk).unwrap();
        let (runtime, collisions) = (&mut self.runtime, &self.collisions);
        let entities = routed_entities(column, None, |id, runtime_id, nbt, position| {
            let info = block_info(
                runtime,
                collisions,
                assets::NetworkIdMode::Sequential,
                runtime_id,
            )?;
            describe(
                id,
                info.backing?,
                &info.name,
                &info.state,
                &nbt.parse()?,
                position,
            )
        });
        let scan = ColumnScan::new(column, Vec::new(), entities);
        self.scans.insert(chunk, scan);
    }

    /// Resolves a cached template against the current world, as the scene does each frame
    /// after rescanning the drawn column; other columns keep their last scan, if any.
    fn pair(&mut self, key: BlockEntityKey, model: ChestModel) -> ChestPair {
        self.scan(key.chunk());
        self.resolve(key, model)
    }

    /// Resolves against the cached scans without rescanning, as a steady frame does.
    fn resolve(&mut self, key: BlockEntityKey, model: ChestModel) -> ChestPair {
        let around = Neighbourhood {
            column: key.chunk(),
            entities: &self.scans[&key.chunk()].entities,
            scans: &self.scans,
            store: &self.store,
            collisions: &self.collisions,
            mode: assets::NetworkIdMode::Sequential,
        };
        let runtime = &mut self.runtime;
        paired_model(model, key, |at| around.chest(at, runtime)).pair
    }
}

/// Two chests facing north side by side, straddling a column boundary.
const WEST: BlockEntityKey = BlockEntityKey::new(0, 15, 65, 1);
const EAST: BlockEntityKey = BlockEntityKey::new(0, 16, 65, 1);

#[test]
fn placing_a_neighbour_replaces_the_cached_single_without_interaction() {
    let same_column = [
        BlockEntityKey::new(0, 4, 65, 1),
        BlockEntityKey::new(0, 5, 65, 1),
    ];
    // Terrain, the placed chest's record and the old chest's record may arrive in
    // any order before the next rendered frame.
    let orders = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    for [old, placed] in [[WEST, EAST], [EAST, WEST], same_column] {
        for old_leads in [false, true] {
            for update_old in [false, true] {
                if old_leads && !update_old {
                    continue;
                }
                for order in orders {
                    let mut fixture = Fixture::new();
                    for key in [old, placed] {
                        fixture.block(key, "minecraft:chest", "north");
                    }
                    fixture.update_block(placed, false);
                    fixture.actor(old, Record::Plain, false);
                    for key in [old, placed] {
                        fixture.scan(key.chunk());
                    }
                    let single = fixture.template(old);
                    assert_eq!(fixture.pair(old, single), ChestPair::Single);
                    for event in order {
                        match event {
                            0 => fixture.update_block(placed, true),
                            1 => fixture.actor(
                                placed,
                                Record::Pair(old.position(), Some(!old_leads)),
                                false,
                            ),
                            2 if update_old => fixture.actor(
                                old,
                                Record::Pair(placed.position(), Some(old_leads)),
                                false,
                            ),
                            _ => {}
                        }
                    }
                    let old_model = fixture.template(old);
                    let placed_model = fixture.template(placed);
                    // Draw the existing column first, while the placed column still has
                    // its pre-placement scan. This is the ordering that exposed the overlap.
                    let (lead, follower, lead_model, follower_model) = if old_leads {
                        (old, placed, old_model, placed_model)
                    } else {
                        assert_eq!(fixture.pair(old, old_model), ChestPair::Follower);
                        (placed, old, placed_model, old_model)
                    };
                    assert_eq!(
                        fixture.pair(lead, lead_model),
                        ChestPair::Lead {
                            partner: follower.position()
                        },
                    );
                    assert_eq!(fixture.pair(follower, follower_model), ChestPair::Follower);
                }
            }
        }
    }
}

#[test]
fn partner_arriving_later_pairs_from_chunk_or_later_update() {
    // The follower's record may name the lead, as vanilla writes it, or carry nothing.
    for follower_record in [None, Some(false)] {
        for in_chunk in [true, false] {
            for lead_first in [true, false] {
                for (leader, follower) in [(WEST, EAST), (EAST, WEST)] {
                    let mut fixture = Fixture::new();
                    let records = [
                        (leader, Record::Pair(follower.position(), Some(true))),
                        (
                            follower,
                            follower_record.map_or(Record::Plain, |lead| {
                                Record::Pair(leader.position(), Some(lead))
                            }),
                        ),
                    ];
                    let [(first, first_record), (second, second_record)] = if lead_first {
                        records
                    } else {
                        [records[1], records[0]]
                    };
                    fixture.block(first, "minecraft:chest", "north");
                    fixture.actor(first, first_record, in_chunk);
                    let first_model = fixture.template(first);
                    assert_eq!(
                        fixture.pair(first, first_model),
                        ChestPair::Single,
                        "an absent partner leaves a single chest"
                    );
                    fixture.block(second, "minecraft:chest", "north");
                    assert_eq!(
                        fixture.pair(first, first_model),
                        ChestPair::Single,
                        "pairing waits for the partner's block actor"
                    );
                    fixture.actor(second, second_record, in_chunk);
                    let second_model = fixture.template(second);
                    let (lead_model, follower_model) = if lead_first {
                        (first_model, second_model)
                    } else {
                        (second_model, first_model)
                    };
                    assert_eq!(
                        fixture.pair(leader, lead_model),
                        ChestPair::Lead {
                            partner: follower.position()
                        }
                    );
                    assert_eq!(
                        fixture.pair(follower, follower_model),
                        ChestPair::Follower,
                        "the follower draws nothing once the lead pairs with it"
                    );
                }
            }
        }
    }
}

#[test]
fn lead_update_pairs_the_partner_column_scanned_before_it() {
    let mut fixture = Fixture::new();
    for key in [WEST, EAST] {
        fixture.block(key, "minecraft:chest", "north");
        fixture.actor(key, Record::Plain, true);
        fixture.scan(key.chunk());
    }
    let follower = fixture.template(EAST);
    assert_eq!(fixture.pair(EAST, follower), ChestPair::Single);
    fixture.actor(WEST, Record::Pair(EAST.position(), Some(true)), false);
    fixture.scan(WEST.chunk());
    let lead = fixture.template(WEST);
    assert_eq!(
        fixture.pair(WEST, lead),
        ChestPair::Lead {
            partner: EAST.position()
        }
    );
    assert_eq!(fixture.pair(EAST, follower), ChestPair::Follower);
}

#[test]
fn steady_frames_resolve_from_current_scans_without_describing_records() {
    let mut fixture = Fixture::new();
    fixture.block(WEST, "minecraft:chest", "north");
    fixture.actor(WEST, Record::Pair(EAST.position(), Some(true)), true);
    fixture.block(EAST, "minecraft:chest", "north");
    fixture.actor(EAST, Record::Plain, true);
    let (lead, follower) = (fixture.template(WEST), fixture.template(EAST));
    for key in [WEST, EAST] {
        fixture.scan(key.chunk());
    }
    for _ in 0..2 {
        let around = Neighbourhood {
            column: WEST.chunk(),
            entities: &fixture.scans[&WEST.chunk()].entities,
            scans: &fixture.scans,
            store: &fixture.store,
            collisions: &fixture.collisions,
            mode: assets::NetworkIdMode::Sequential,
        };
        for key in [WEST, EAST] {
            assert!(
                around
                    .chest_or_else(key, || panic!("a current scan needs no fallback"))
                    .is_some()
            );
        }
        assert_eq!(
            fixture.resolve(WEST, lead),
            ChestPair::Lead {
                partner: EAST.position()
            }
        );
        assert_eq!(fixture.resolve(EAST, follower), ChestPair::Follower);
    }
}

#[test]
fn claims_resolve_to_exactly_one_lead() {
    // A record without `pairlead` leads, so both halves may claim each other.
    for flag in [None, Some(true)] {
        let mut fixture = Fixture::new();
        fixture.block(WEST, "minecraft:chest", "north");
        fixture.actor(WEST, Record::Pair(EAST.position(), flag), true);
        fixture.block(EAST, "minecraft:chest", "north");
        fixture.actor(EAST, Record::Pair(WEST.position(), flag), true);
        let (west, east) = (fixture.template(WEST), fixture.template(EAST));
        let pairs = [fixture.pair(WEST, west), fixture.pair(EAST, east)];
        assert_eq!(
            pairs
                .iter()
                .filter(|pair| matches!(pair, ChestPair::Lead { .. }))
                .count(),
            1
        );
        assert!(pairs.contains(&ChestPair::Follower));
    }
    // Two chests in one column claiming the same partner: only one draws over it.
    let mut fixture = Fixture::new();
    let middle = BlockEntityKey::new(0, 5, 65, 1);
    let sides = [
        BlockEntityKey::new(0, 4, 65, 1),
        BlockEntityKey::new(0, 6, 65, 1),
    ];
    fixture.block(middle, "minecraft:chest", "north");
    fixture.actor(middle, Record::Plain, false);
    for side in sides {
        fixture.actor(side, Record::Pair(middle.position(), Some(true)), false);
    }
    let leads = sides
        .map(|side| {
            let model = fixture.template(side);
            fixture.pair(side, model)
        })
        .into_iter()
        .filter(|pair| matches!(pair, ChestPair::Lead { .. }))
        .count();
    assert_eq!(leads, 1);
    let model = fixture.template(middle);
    assert_eq!(fixture.pair(middle, model), ChestPair::Follower);
}

#[test]
fn non_leading_record_alone_stays_single() {
    let mut fixture = Fixture::new();
    fixture.block(WEST, "minecraft:chest", "north");
    fixture.actor(WEST, Record::Plain, true);
    fixture.block(EAST, "minecraft:chest", "north");
    fixture.actor(EAST, Record::Pair(WEST.position(), Some(false)), true);
    for key in [WEST, EAST] {
        let model = fixture.template(key);
        assert_eq!(fixture.pair(key, model), ChestPair::Single);
    }
}

#[test]
fn pairing_needs_compatible_blocks_beside_the_lead() {
    for (name, facing) in [
        ("minecraft:chest", "south"),
        ("minecraft:chest", "east"),
        ("minecraft:trapped_chest", "north"),
    ] {
        let mut fixture = Fixture::new();
        fixture.block(WEST, "minecraft:chest", "north");
        fixture.actor(WEST, Record::Pair(EAST.position(), Some(true)), true);
        fixture.block(EAST, name, facing);
        fixture.actor(EAST, Record::Plain, true);
        let (lead, other) = (fixture.template(WEST), fixture.template(EAST));
        assert_eq!(
            fixture.pair(WEST, lead),
            ChestPair::Single,
            "{name} {facing}"
        );
        assert_eq!(
            fixture.pair(EAST, other),
            ChestPair::Single,
            "{name} {facing}"
        );
    }
    // East-facing chests pair along z, never along x.
    let mut fixture = Fixture::new();
    fixture.block(WEST, "minecraft:chest", "east");
    fixture.actor(WEST, Record::Pair(EAST.position(), Some(true)), true);
    fixture.block(EAST, "minecraft:chest", "east");
    fixture.actor(EAST, Record::Plain, true);
    let lead = fixture.template(WEST);
    assert_eq!(fixture.pair(WEST, lead), ChestPair::Single);
}
