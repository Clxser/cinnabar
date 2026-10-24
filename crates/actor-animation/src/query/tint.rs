//! Native dye defaults inherited by entity render controllers.
use super::{ActorMetadataValue, AnimatedActor, actor_flag};
use assets::facts::entity::NativeEntityClass;
use protocol_ids::{ActorDataKey, ActorFlag};

/// Supplies sheep wool and tamed wolf collar color when no authored color overrides it.
pub(crate) fn default_render_color(actor: &dyn AnimatedActor) -> [f32; 4] {
    match actor.entity_fact().native_class {
        NativeEntityClass::Sheep => {}
        NativeEntityClass::Wolf if actor_flag(actor, ActorFlag::Tamed.wire_value()) => {}
        _ => return [1.0; 4],
    }
    let index = match actor.metadata().get(&ActorDataKey::ColorIndex.wire_value()) {
        Some(ActorMetadataValue::Byte(value)) => *value as u8,
        _ => 0,
    };
    assets::dye::actor_palette_color(index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_actor::TestActor;
    use protocol::ActorKind;
    use std::collections::HashMap;

    /// Makes an actor with the exact metadata kind supplied by the test.
    fn actor(identifier: &str, color: ActorMetadataValue) -> TestActor {
        TestActor {
            kind: ActorKind::Entity {
                identifier: identifier.into(),
            },
            metadata: HashMap::from([(ActorDataKey::ColorIndex.wire_value(), color)]),
            ..TestActor::default()
        }
    }

    #[test]
    fn dyed_sheep_inherit_their_synced_wool_color() {
        for color in 0..=255u8 {
            let sheep = actor("minecraft:sheep", ActorMetadataValue::Byte(color as i8));
            assert_eq!(
                default_render_color(&sheep),
                assets::dye::actor_palette_color(color)
            );
        }
    }

    #[test]
    fn missing_or_wrong_metadata_kind_keeps_sheep_white() {
        let mut sheep = actor("minecraft:sheep", ActorMetadataValue::Int(7));
        assert_eq!(default_render_color(&sheep), [1.0; 4]);
        sheep.metadata.clear();
        assert_eq!(default_render_color(&sheep), [1.0; 4]);
    }

    #[test]
    fn only_tamed_wolves_acquire_collar_dye_and_other_actors_stay_neutral() {
        let mut wolf = actor("minecraft:wolf", ActorMetadataValue::Byte(7));
        assert_eq!(default_render_color(&wolf), [1.0; 4]);
        wolf.metadata.insert(
            ActorDataKey::Flags.wire_value(),
            ActorMetadataValue::Flags(1 << ActorFlag::Tamed.wire_value()),
        );
        assert_eq!(
            default_render_color(&wolf),
            assets::dye::actor_palette_color(7)
        );
        wolf.kind = ActorKind::Entity {
            identifier: "minecraft:cow".into(),
        };
        assert_eq!(default_render_color(&wolf), [1.0; 4]);
    }

    #[test]
    fn custom_actor_with_sheep_base_inherits_the_dye_rule() {
        let mut sheep = actor("fixture:wool", ActorMetadataValue::Byte(7));
        sheep.entity_fact_override =
            Some(assets::facts::entity::EntityFacts::pinned().get("minecraft:sheep"));
        assert_eq!(
            default_render_color(&sheep),
            assets::dye::actor_palette_color(7)
        );
    }
}
