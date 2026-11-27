use sky_data::generated_entities::id as entity_id;
use sky_data::generated_items;
use sky_data::generated_items::id as item_id;
use uuid::Uuid;

/// Floating dropped item stack in the Overworld (`minecraft:item` entity) awaiting pickup.
#[derive(Debug, Clone)]
pub struct WorldItemDrop {
    pub entity_id: i32,
    pub uuid: Uuid,
    pub item_id: u32,
    pub count: u8,
    pub position: [f64; 3],
    pub spawn_tick: u64,
}

impl WorldItemDrop {
    pub const PROTOCOL_ENTITY_TYPE: i32 = entity_id::ITEM as i32;

    pub fn new(
        entity_id: i32,
        item_id: u32,
        count: u8,
        position: [f64; 3],
        spawn_tick: u64,
    ) -> Self {
        Self {
            entity_id,
            uuid: Uuid::new_v4(),
            item_id,
            count,
            position,
            spawn_tick,
        }
    }
}

/// Resolves the Survival item drop `(item_id, count)` produced when mining a block `block_key`.
pub fn block_survival_drop(block_key: &str) -> Option<(u32, u8)> {
    match block_key {
        "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air" | "minecraft:water"
        | "minecraft:lava" | "minecraft:bedrock" | "minecraft:glass" => None,
        "minecraft:stone" => Some((item_id::COBBLESTONE, 1)),
        "minecraft:grass_block" => Some((item_id::DIRT, 1)),
        "minecraft:coal_ore" | "minecraft:deepslate_coal_ore" => Some((item_id::COAL, 1)),
        "minecraft:iron_ore" | "minecraft:deepslate_iron_ore" => Some((item_id::RAW_IRON, 1)),
        "minecraft:gold_ore" | "minecraft:deepslate_gold_ore" => Some((item_id::GOLD_INGOT, 1)),
        "minecraft:diamond_ore" | "minecraft:deepslate_diamond_ore" => Some((item_id::DIAMOND, 1)),
        "minecraft:redstone_ore" | "minecraft:deepslate_redstone_ore" => {
            Some((item_id::REDSTONE, 4))
        }
        "minecraft:oak_leaves" => Some((item_id::APPLE, 1)),
        other => generated_items::by_key(other).map(|def| (def.id, 1)),
    }
}
