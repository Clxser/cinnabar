//! Client-predicted sounds tied to local interaction: block place/hit/break, eating and drinking,
//! the player's own attacks, actor hurt/death status events, and dropped-item pickup.

use std::sync::Arc;

use bevy::prelude::{Local, Message, MessageReader, MessageWriter, Res, ResMut, Time};
use client_world::ActorStatusNotice;
use protocol::{ActorKind, ActorStatusKind};
use sim::PaletteWorld;

use super::{
    echo::{EchoOrigin, EchoSubject},
    engine::{AudioEngine, SoundRequest},
    systems::{ACTOR_ECHO_SECONDS, BLOCK_ECHO_SECONDS, block_lookup, identifier_ref},
};

const PLAYER: &str = assets::facts::VanillaEntity::Player.identifier();
/// Height fraction of an actor's box standing in for its head attach point.
const HEAD_HEIGHT_FRACTION: f32 = 0.9;
/// Seconds between block hit sounds while mining (vanilla 200 ms).
const HIT_INTERVAL: f32 = 0.2;

/// One bite of the local player's food, in step with its eating sound; particles scatter the
/// item's crumbs from it.
#[derive(Debug, Clone, PartialEq, Eq, Message)]
pub struct LocalEatingCue {
    pub identifier: Arc<str>,
    pub metadata: u32,
}

/// A local block interaction the audio runtime should voice before the server confirms it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Message)]
pub enum LocalBlockCue {
    /// A locally completed destroy; preserve its id before prediction replaces it with air.
    Break {
        position: [i32; 3],
        block_runtime_id: i32,
    },
    /// A local placement voiced at `position`, which must match the cell of the server's copy
    /// so the two pair up. The id uses the local store palette.
    Place {
        position: [i32; 3],
        block_runtime_id: i32,
    },
}

/// The local player's attack sound, voiced on the frame the attack is sent. Servers send
/// it only to other players, so no server copy is expected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Message)]
pub struct LocalAttackCue {
    /// The player's level sound event, such as `attack.strong`.
    pub event: &'static str,
}

#[derive(Debug, Default)]
pub struct MiningAudio {
    target: Option<([i32; 3], Option<String>, Option<String>)>,
    hit_timer: f32,
}

/// A predicted block sound with the material captured before the world changes.
struct BlockSoundCue {
    event: &'static str,
    identifier: Option<String>,
    component_material: Option<String>,
    cell: [i32; 3],
}

fn center(cell: [i32; 3]) -> [f32; 3] {
    cell.map(|axis| axis as f32 + 0.5)
}

/// Selects the sound event from the same admitted consumption fact as first-person animation.
pub fn consume_event(consumption: assets::item_use::ItemConsumption) -> Option<&'static str> {
    match consumption {
        assets::item_use::ItemConsumption::Eat => Some("eat"),
        assets::item_use::ItemConsumption::Drink => Some("drink"),
        assets::item_use::ItemConsumption::Honey => Some("drink.honey"),
        assets::item_use::ItemConsumption::Unresolved | assets::item_use::ItemConsumption::None => {
            None
        }
    }
}

/// Whether the admitted use scatters food crumbs through local eating cues.
pub fn predicts_bites(consumption: assets::item_use::ItemConsumption) -> bool {
    matches!(
        actor_animation::consumables::consume_animation(consumption),
        Some(actor_animation::consumables::ConsumeAnimation::Eat)
    )
}

/// Voices local block placement, mining hits and the predicted break of the mined block.
#[allow(clippy::too_many_arguments)]
pub fn drive_block_cues(
    mut cues: MessageReader<LocalBlockCue>,
    time: Res<Time>,
    world: crate::observations::WorldObservation<'_>,
    collisions: Option<&dyn crate::observations::CollisionLookup>,
    survival: Option<&dyn crate::observations::MiningObservation>,
    mut engine: ResMut<AudioEngine>,
    mut mining: Local<MiningAudio>,
) {
    let (Some(stream), Some(collisions)) = (world.stream.as_ref(), collisions) else {
        cues.clear();
        *mining = MiningAudio::default();
        return;
    };
    if !engine.has_bank() {
        cues.clear();
        return;
    }
    let mode = stream.network_id_mode();
    let lookup = block_lookup(Some(collisions), mode);
    let palette = PaletteWorld::new(
        stream.collision_store(),
        collisions.registry(mode),
        stream.current_dimension(),
        Default::default(),
    );
    let mut requests = Vec::new();
    for cue in cues.read() {
        let (event, position, block_runtime_id) = match *cue {
            LocalBlockCue::Place {
                position,
                block_runtime_id,
            } => ("place", position, block_runtime_id),
            LocalBlockCue::Break {
                position,
                block_runtime_id,
            } => ("break", position, block_runtime_id),
        };
        let runtime_id = block_runtime_id as u32;
        requests.push(BlockSoundCue {
            event,
            identifier: lookup(runtime_id),
            component_material: collisions
                .block_sound_material(mode, runtime_id)
                .map(str::to_owned),
            cell: position,
        });
    }
    let target = survival
        .and_then(|survival| survival.destroying_target())
        .map(|(cell, _face)| cell);
    let previous = mining.target.as_ref().map(|(cell, _, _)| *cell);
    if target != previous {
        mining.hit_timer = 0.0;
        mining.target = target.map(|cell| {
            let runtime_id = palette.primary_runtime_id(cell).ok();
            let identifier = identifier_ref(&palette, collisions, mode, cell).map(str::to_owned);
            let component_material = runtime_id
                .and_then(|runtime_id| collisions.block_sound_material(mode, runtime_id))
                .map(str::to_owned);
            (cell, identifier, component_material)
        });
    }
    if let Some(cell) = mining.target.as_ref().map(|(cell, _, _)| *cell) {
        mining.hit_timer -= time.delta_secs();
        if mining.hit_timer <= 0.0 {
            mining.hit_timer = HIT_INTERVAL;
            let (identifier, component_material) = mining
                .target
                .as_ref()
                .map(|(_, identifier, component_material)| {
                    (identifier.clone(), component_material.clone())
                })
                .unwrap_or_default();
            requests.push(BlockSoundCue {
                event: "hit",
                identifier,
                component_material,
                cell,
            });
        }
    }
    let built: Vec<(&'static str, [i32; 3], SoundRequest)> = {
        let Some(bank) = engine.bank() else { return };
        let tables = bank.tables();
        requests
            .into_iter()
            .filter_map(
                |BlockSoundCue {
                     event,
                     identifier,
                     component_material,
                     cell,
                 }| {
                    let identifier = identifier?;
                    let material = tables
                        .material_of_with_component(&identifier, component_material.as_deref());
                    let route = tables.block(material, event)?;
                    let request = SoundRequest::new(route.sound)
                        .with_ranges(route.volume, route.pitch)
                        .at(center(cell));
                    Some((event, cell, request))
                },
            )
            .collect()
    };
    for (event, cell, request) in built {
        if event == "hit"
            || engine.admit_echo(
                EchoOrigin::Client,
                event,
                EchoSubject::Cell(cell),
                BLOCK_ECHO_SECONDS,
            )
        {
            engine.enqueue(request);
        }
    }
}

#[derive(Debug, Default)]
pub struct ConsumeAudio {
    item: Option<Arc<str>>,
    started_tick: Option<u64>,
    elapsed_tick: Option<u32>,
}

/// The admitted consumable and timing that owned the completed movement tick.
pub struct ConsumeUse {
    pub identifier: Arc<str>,
    pub metadata: u32,
    pub consumption: assets::item_use::ItemConsumption,
    pub started_tick: u64,
    pub duration: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConsumeCue {
    /// The player sound event of one eating/drinking step.
    Step(&'static str),
    /// The use ran its full duration; food finishes with a burp.
    Finished,
}

impl ConsumeAudio {
    /// Emits crumbs only for an admitted food use with an eating step in this frame.
    fn eating_cue(
        &self,
        consumption: assets::item_use::ItemConsumption,
        metadata: u32,
        cues: &[ConsumeCue],
    ) -> Option<LocalEatingCue> {
        if !predicts_bites(consumption) || !cues.contains(&ConsumeCue::Step("eat")) {
            return None;
        }
        Some(LocalEatingCue {
            identifier: Arc::clone(self.item.as_ref()?),
            metadata,
        })
    }

    /// Emits each newly reached use-tick cue once, including when several ticks share a frame.
    fn advance(
        &mut self,
        item: Option<(Arc<str>, &'static str)>,
        timing: Option<(u64, u32)>,
        tick: u64,
    ) -> Vec<ConsumeCue> {
        let identifier = item.as_ref().map(|(identifier, _)| Arc::clone(identifier));
        let started_tick = timing.map(|(start, _)| start);
        if self.item != identifier || self.started_tick != started_tick {
            *self = Self {
                item: identifier,
                started_tick,
                ..Self::default()
            };
        }
        let (Some((_, event)), Some((start, duration))) = (item, timing) else {
            return Vec::new();
        };
        if duration == 0 {
            return Vec::new();
        }
        let Some(elapsed) = tick
            .checked_sub(start)
            .map(|elapsed| elapsed.saturating_add(1).min(u64::from(duration)) as u32)
        else {
            return Vec::new();
        };
        let first = self
            .elapsed_tick
            .map_or(1, |previous| previous.saturating_add(1));
        self.elapsed_tick = Some(elapsed);
        let mut cues = Vec::new();
        // Only the final window can emit, even after a long gap between observations.
        for used in first
            .max(duration.saturating_sub(assets::item_use::CONSUME_EFFECT_WINDOW_TICKS))
            ..=elapsed
        {
            let remaining = duration - used;
            if assets::item_use::consume_effect_tick(remaining) {
                cues.push(ConsumeCue::Step(event));
            }
            if remaining == 0 && event == "eat" {
                cues.push(ConsumeCue::Finished);
            }
        }
        cues
    }
}

/// Voices an admitted consumable's completed ticks, including its final bite and food burp.
pub fn drive_consume_audio(
    admitted: Option<ConsumeUse>,
    tick: u64,
    eye: bevy::prelude::Vec3,
    mut engine: ResMut<AudioEngine>,
    mut state: Local<ConsumeAudio>,
    mut eating: MessageWriter<LocalEatingCue>,
) {
    let use_timing = admitted
        .as_ref()
        .map(|use_| (use_.started_tick, use_.duration));
    let metadata = admitted.as_ref().map_or(0, |use_| use_.metadata);
    let consumption = admitted
        .as_ref()
        .map_or_else(Default::default, |use_| use_.consumption);
    let item = admitted.and_then(|use_| Some((use_.identifier, consume_event(use_.consumption)?)));
    let cues = state.advance(item, use_timing, tick);
    if cues.is_empty() {
        return;
    }
    if let Some(cue) = state.eating_cue(consumption, metadata, &cues) {
        for _ in cues.iter().filter(|cue| **cue == ConsumeCue::Step("eat")) {
            eating.write(cue.clone());
        }
    }
    let requests: Vec<SoundRequest> = {
        let Some(bank) = engine.bank() else { return };
        let tables = bank.tables();
        cues.iter()
            .filter_map(|cue| match cue {
                ConsumeCue::Step(event) => tables.entity(PLAYER, event, None),
                ConsumeCue::Finished => tables.individual("burp").cloned(),
            })
            .map(|route| {
                SoundRequest::new(route.sound)
                    .with_ranges(route.volume, route.pitch)
                    .at([eye.x, eye.y, eye.z])
            })
            .collect()
    };
    for request in requests {
        engine.enqueue(request);
    }
}

/// Voices the local player's attack cues at `position`, unless the player is silent.
pub fn drive_attack_cues(
    mut cues: MessageReader<LocalAttackCue>,
    world: crate::observations::WorldObservation<'_>,
    position: bevy::prelude::Vec3,
    mut engine: ResMut<AudioEngine>,
) {
    use actor_animation::AnimatedActor;
    let silent = world.stream.as_ref().is_none_or(|stream| {
        stream
            .authority()
            .actor(stream.local_player_runtime_id())
            .is_some_and(|actor| actor.flag(protocol_ids::ActorFlag::Silent.wire_value()))
    });
    let requests: Vec<SoundRequest> = match engine.bank() {
        Some(bank) if !silent => cues
            .read()
            .filter_map(|cue| attack_request(bank.tables(), *cue, position.into()))
            .collect(),
        _ => {
            cues.clear();
            return;
        }
    };
    for request in requests {
        engine.enqueue(request);
    }
}

/// The player's sound for an attack cue at `position`.
fn attack_request(
    tables: &assets::SoundEventTables,
    cue: LocalAttackCue,
    position: [f32; 3],
) -> Option<SoundRequest> {
    let route = tables.entity(PLAYER, cue.event, None)?;
    Some(
        SoundRequest::new(route.sound)
            .with_ranges(route.volume, route.pitch)
            .at(position),
    )
}

/// Hurt or death sound of `identifier` for a status `notice`, from the actor's head.
fn status_request(
    tables: &assets::SoundEventTables,
    identifier: &str,
    class: assets::facts::entity::NativeEntityClass,
    notice: &ActorStatusNotice,
) -> Option<(&'static str, SoundRequest)> {
    let event = match notice.kind {
        ActorStatusKind::Hurt | ActorStatusKind::HurtWithoutDamage => "hurt",
        ActorStatusKind::Death
            if class == assets::facts::entity::NativeEntityClass::EnderDragon =>
        {
            return None;
        }
        ActorStatusKind::Death => "death",
        _ => return None,
    };
    let route = tables.entity(identifier, event, None)?;
    let mut head = notice.position;
    head[1] += notice.height.unwrap_or(0.0).max(0.0) * HEAD_HEIGHT_FRACTION;
    let request = SoundRequest::new(route.sound)
        .with_ranges(route.volume, route.pitch)
        .at(head);
    Some((event, request))
}

/// Voices every actor's hurt/death status.
pub fn drive_actor_audio(
    world: crate::observations::WorldObservation<'_>,
    inbox: Option<&mut dyn crate::observations::ParticleAudioObservation>,
    mut engine: ResMut<AudioEngine>,
) {
    let Some(stream) = world.stream.as_ref() else {
        if let Some(inbox) = inbox {
            inbox.take_status_audio();
        }
        return;
    };
    let notices = inbox
        .map(|inbox| inbox.take_status_audio())
        .unwrap_or_default();
    if !engine.has_bank() {
        return;
    }
    let local = stream.local_player_runtime_id();
    for notice in &notices {
        let (identifier, unique_id, class) = if notice.runtime_id == local {
            (
                PLAYER,
                stream.authority().local_player_unique_id(),
                assets::facts::entity::NativeEntityClass::Player,
            )
        } else {
            let Some(actor) = stream.authority().actor(notice.runtime_id) else {
                continue;
            };
            let identifier = match &actor.kind {
                ActorKind::Player { .. } => PLAYER,
                ActorKind::Entity { identifier } => identifier.as_ref(),
            };
            (identifier, actor.unique_id, actor.entity_fact.native_class)
        };
        let Some((event, request)) = engine
            .bank()
            .and_then(|bank| status_request(bank.tables(), identifier, class, notice))
        else {
            continue;
        };
        let subject = EchoSubject::Actor(unique_id);
        if engine.admit_echo(EchoOrigin::Client, event, subject, ACTOR_ECHO_SECONDS) {
            engine.enqueue(request);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Resolves an original compatibility fixture before checking its consumption sound.
    fn fixture_consume_event(identifier: &str) -> Option<&'static str> {
        let profile = assets::item_use::diagnostic_profile(identifier);
        consume_event(client_world::resolve_item_use(&profile, Some(32), None).consumption)
    }

    #[test]
    fn consumables_are_classified() {
        assert_eq!(fixture_consume_event("minecraft:bread"), Some("eat"));
        assert_eq!(fixture_consume_event("minecraft:potion"), Some("drink"));
        assert_eq!(
            fixture_consume_event("minecraft:honey_bottle"),
            Some("drink.honey")
        );
        assert_eq!(
            fixture_consume_event("minecraft:experience_bottle"),
            None,
            "thrown"
        );
        assert_eq!(fixture_consume_event("minecraft:stone"), None);
    }

    #[test]
    fn only_admitted_eating_uses_predict_local_bites() {
        use assets::item_use::ItemConsumption;
        assert!(predicts_bites(ItemConsumption::Eat));
        for consumption in [
            ItemConsumption::Drink,
            ItemConsumption::Honey,
            ItemConsumption::None,
            ItemConsumption::Unresolved,
        ] {
            assert!(!predicts_bites(consumption));
        }
    }

    #[test]
    fn vanilla_food_overridden_to_drink_emits_no_crumbs() {
        use assets::item_use::ItemConsumption;
        use chunk_pipeline::WorldStream;

        for server_override in [false, true] {
            let mut stream = WorldStream::new(protocol::WorldBootstrap {
                dimension: 0,
                local_player_runtime_id: 1,
                local_player_unique_id: 1,
                player_position: [0.0; 3],
                world_spawn_position: [0; 3],
                air_network_id: protocol_ids::SEQUENTIAL_AIR_NETWORK_ID,
                block_network_ids_are_hashes: false,
            });
            let identifier: Arc<str> = "minecraft:bread".into();
            let network_id = 15000;
            assert!(stream.seed_item_registry(protocol::ItemRegistryEvent {
                entries: Arc::from([protocol::ItemRegistryEntry {
                    identifier: Arc::clone(&identifier),
                    network_id,
                    component_based: true,
                    version: protocol::ItemRegistryVersion::DataDriven,
                    component_digest: [0; 32],
                    components: Some(Arc::new(protocol::ItemComponents {
                        use_duration_ticks: Some(32),
                        use_animation: server_override.then(|| "drink".into()),
                        ..Default::default()
                    })),
                    negotiated_max_stack_size: None,
                    canonical_empty_component_data: false,
                    item_tags: Arc::from([]),
                }]),
            }));
            let mut state = ConsumeAudio::default();
            if !server_override {
                let base = stream.authority().item_use_profile(network_id);
                assert_eq!(base.consumption, ItemConsumption::Eat);
                let cues = state.advance(
                    Some((
                        Arc::clone(&identifier),
                        consume_event(base.consumption).unwrap(),
                    )),
                    Some((0, base.ticks.unwrap())),
                    8,
                );
                assert!(state.eating_cue(base.consumption, 0, &cues).is_some());
                stream.set_item_use_overlays(Arc::new(std::collections::BTreeMap::from([(
                    identifier.as_ref().into(),
                    assets::equipment::CompiledItemUseOverlay {
                        identifier: identifier.as_ref().into(),
                        consumption: Some(ItemConsumption::Drink),
                        ..Default::default()
                    },
                )])));
            }
            let effective = stream.authority().item_use_profile(network_id);
            assert_eq!(effective.consumption, ItemConsumption::Drink);
            let cues = state.advance(
                Some((
                    Arc::clone(&identifier),
                    consume_event(effective.consumption).unwrap(),
                )),
                Some((0, effective.ticks.unwrap())),
                12,
            );
            assert!(cues.contains(&ConsumeCue::Step("drink")));
            assert!(state.eating_cue(effective.consumption, 0, &cues).is_none());
        }
    }

    #[test]
    fn consume_cues_wait_for_the_first_bite_tick_of_each_duration() {
        // Use tick 1 is the tick the use starts; vanilla bites once 25 or fewer ticks remain.
        for (identifier, event, duration, steps) in [
            (
                "minecraft:golden_apple",
                "eat",
                32,
                &[8, 12, 16, 20, 24, 28, 32][..],
            ),
            ("minecraft:dried_kelp", "eat", 16, &[4, 8, 12, 16][..]),
            (
                "minecraft:honey_bottle",
                "drink.honey",
                40,
                &[16, 20, 24, 28, 32, 36, 40][..],
            ),
        ] {
            let item = || Some((Arc::from(identifier), event));
            let mut state = ConsumeAudio::default();
            let mut stepped = Vec::new();
            for used in 1..=duration {
                let tick = 100 + u64::from(used) - 1;
                let cues = state.advance(item(), Some((100, duration)), tick);
                if cues.contains(&ConsumeCue::Step(event)) {
                    stepped.push(used);
                }
                if event == "eat" {
                    assert_eq!(
                        state
                            .eating_cue(assets::item_use::ItemConsumption::Eat, 0, &cues)
                            .is_some(),
                        cues.contains(&ConsumeCue::Step(event)),
                        "{identifier} crumbs follow its bite sound"
                    );
                }
                assert!(
                    state
                        .advance(item(), Some((100, duration)), tick)
                        .is_empty(),
                    "{identifier} repeated a cue within one tick"
                );
            }
            assert_eq!(stepped, steps, "{identifier}");
        }
    }

    #[test]
    fn short_food_uses_its_remaining_ticks_and_restarting_resets_cues() {
        let kelp = || Some((Arc::from("minecraft:dried_kelp"), "eat"));
        let mut state = ConsumeAudio::default();
        assert_eq!(
            state.advance(kelp(), Some((100, 16)), 103),
            [ConsumeCue::Step("eat")]
        );
        assert!(state.advance(kelp(), Some((100, 16)), 104).is_empty());
        assert_eq!(
            state.advance(kelp(), Some((100, 16)), 111),
            [ConsumeCue::Step("eat"), ConsumeCue::Step("eat")]
        );
        assert_eq!(
            state.advance(kelp(), Some((100, 16)), 115),
            [ConsumeCue::Step("eat"), ConsumeCue::Finished]
        );
        assert!(state.advance(kelp(), Some((100, 16)), 117).is_empty());
        assert!(state.advance(None, None, 118).is_empty());
        assert_eq!(
            state.advance(kelp(), Some((120, 16)), 123),
            [ConsumeCue::Step("eat")]
        );
    }

    // Only the local player's status was voiced; remote players and mobs stayed silent.
    #[test]
    fn remote_actor_status_voices_its_own_hurt_sound_from_the_head() {
        let tables = assets::SoundEventTables::from_json(
            &serde_json::json!({"entity_sounds": {
                "defaults": {"events": {"hurt": "game.hurt", "death": "game.death"}},
                "entities": {"zombie": {"events": {"hurt": "mob.zombie.hurt"}},
                    "armor_stand": {"events": {"hurt": ""}}}}}),
            &serde_json::json!({}),
        );
        let notice = |kind| ActorStatusNotice {
            runtime_id: 9,
            kind,
            data: 0,
            position: [1.0, 64.0, 1.0],
            height: Some(2.0),
            eating_position: [0.0; 3],
        };
        let (event, hurt) = status_request(
            &tables,
            "minecraft:zombie",
            assets::facts::entity::EntityFacts::pinned()
                .get("minecraft:zombie")
                .native_class,
            &notice(ActorStatusKind::Hurt),
        )
        .unwrap();
        assert_eq!((event, &*hurt.name), ("hurt", "mob.zombie.hurt"));
        assert_eq!(hurt.position, Some([1.0, 65.8, 1.0]));
        let (_, death) = status_request(
            &tables,
            "minecraft:zombie",
            assets::facts::entity::EntityFacts::pinned()
                .get("minecraft:zombie")
                .native_class,
            &notice(ActorStatusKind::Death),
        )
        .unwrap();
        assert_eq!(&*death.name, "game.death");
        let silent = status_request(
            &tables,
            "minecraft:armor_stand",
            assets::facts::entity::EntityFacts::pinned()
                .get("minecraft:armor_stand")
                .native_class,
            &notice(ActorStatusKind::Hurt),
        );
        assert!(silent.is_none());
    }

    #[test]
    fn dragon_death_audio_waits_for_the_server_sound_event() {
        let tables = assets::SoundEventTables::from_json(
            &serde_json::json!({"entity_sounds": {"entities": {
                "ender_dragon": {"volume": 80.0, "pitch": [0.8, 1.2], "events": {
                    "death": "mob.enderdragon.death", "hurt": "mob.enderdragon.hit"
                }}
            }}}),
            &serde_json::json!({}),
        );
        let notice = ActorStatusNotice {
            runtime_id: 9,
            kind: ActorStatusKind::Death,
            data: 0,
            position: [1.0, 64.0, 1.0],
            height: Some(4.0),
            eating_position: [0.0; 3],
        };
        assert!(
            status_request(
                &tables,
                "minecraft:ender_dragon",
                assets::facts::entity::EntityFacts::pinned()
                    .get("minecraft:ender_dragon")
                    .native_class,
                &notice
            )
            .is_none(),
            "the death status starts the visual sequence without an early voice"
        );
        let hurt = ActorStatusNotice {
            kind: ActorStatusKind::Hurt,
            ..notice
        };
        assert!(
            status_request(
                &tables,
                "minecraft:ender_dragon",
                assets::facts::entity::EntityFacts::pinned()
                    .get("minecraft:ender_dragon")
                    .native_class,
                &hurt
            )
            .is_some()
        );
        let event = protocol::LevelAudioEvent {
            sound_event: "death".into(),
            position: [1.0, 64.2, 1.0],
            data: -1,
            actor_identifier: "minecraft:ender_dragon".into(),
            is_baby: false,
            is_global: false,
            actor_unique_id: -1,
            fire_at_position: None,
        };
        let request = super::super::route::level_sound_request(&tables, &event, &|_| None)
            .expect("the explicit server event owns the death voice");
        assert_eq!(&*request.name, "mob.enderdragon.death");
        assert_eq!(request.position, Some(event.position));
        assert_eq!(
            request.volume,
            assets::FloatRange {
                min: 80.0,
                max: 80.0
            }
        );
        assert_eq!(request.pitch, assets::FloatRange { min: 0.8, max: 1.2 });
    }

    /// Vanilla spaces mining hit sounds 200 ms apart.
    #[test]
    fn mining_hits_follow_the_reference_interval() {
        assert_eq!(HIT_INTERVAL, 0.2);
    }

    #[test]
    fn cell_centers_offset_by_half_a_block() {
        assert_eq!(center([1, -2, 3]), [1.5, -1.5, 3.5]);
    }

    #[test]
    fn attack_cues_voice_the_player_attack_sound_at_the_player() {
        let tables = assets::SoundEventTables::from_json(
            &serde_json::json!({"entity_sounds": {"entities": {"player": {
                "volume": 1.0, "pitch": [0.8, 1.2], "events": {
                    "attack.critical": "game.player.attack.critical",
                    "attack.strong": "game.player.attack.strong"}}}}}),
            &serde_json::json!({}),
        );
        let request = attack_request(
            &tables,
            LocalAttackCue {
                event: "attack.critical",
            },
            [1.0, 65.62, 1.0],
        )
        .unwrap();
        assert_eq!(&*request.name, "game.player.attack.critical");
        assert_eq!(request.position, Some([1.0, 65.62, 1.0]));
        let missing = LocalAttackCue {
            event: "attack.nodamage",
        };
        assert!(attack_request(&tables, missing, [0.0; 3]).is_none());
    }
}
