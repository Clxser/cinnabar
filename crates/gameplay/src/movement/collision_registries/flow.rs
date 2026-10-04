//! Native material/flow cache bindings; geometry flags do not establish them.

use assets::{ModelStateField, RegistryRecord};
use sim::{CollisionRegistry, FlowBlockFacts};

pub(super) fn register(registry: &mut CollisionRegistry, runtime_id: u32, record: &RegistryRecord) {
    let (blocks_motion, is_solid, liquid) = match record.name.as_ref() {
        // Air is passable; water and lava are liquids.
        "minecraft:air" => (false, false, false),
        "minecraft:water"
        | "minecraft:flowing_water"
        | "minecraft:lava"
        | "minecraft:flowing_lava" => (false, false, true),
        // Dirt and grass block movement.
        "minecraft:dirt" | "minecraft:grass" | "minecraft:grass_block" => (true, true, false),
        // Both ice variants are solid.
        "minecraft:ice" | "minecraft:packed_ice" => (true, true, false),
        // Stone and sand are solid.
        "minecraft:stone" | "minecraft:sand" => (true, true, false),
        // Gravel is solid.
        "minecraft:gravel" => (true, true, false),
        _ => return,
    };
    let liquid_depth = if liquid {
        let Some(depth) = record
            .model_state
            .get(ModelStateField::LiquidDepth)
            .and_then(|depth| u8::try_from(depth).ok())
            .filter(|depth| *depth < 16)
        else {
            return;
        };
        Some(depth)
    } else {
        None
    };
    registry.set_flow_facts(
        runtime_id,
        FlowBlockFacts {
            blocks_motion,
            is_solid,
            // These materials permit flow across every face and have no
            // blocked faces in the liquid-flow cache.
            blocked_faces: 0,
            allowed_faces: 0x3f,
            liquid_depth,
        },
    );
}
