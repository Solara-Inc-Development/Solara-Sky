use crate::item_stack::ItemStack;
use sky_data::generated_items::id as item_id;

#[derive(Debug, Clone, Copy)]
pub struct SimpleRecipe {
    pub name: &'static str,
    pub input_a: (u32, u8),
    pub input_b: Option<(u32, u8)>,
    pub output: ItemStack,
}

/// 16 essential playable Survival crafting recipes usable via the `E` Crafting UI and `/craft`.
pub static CORE_RECIPES: &[SimpleRecipe] = &[
    SimpleRecipe {
        name: "oak_planks",
        input_a: (item_id::OAK_LOG, 1),
        input_b: None,
        output: ItemStack::new(item_id::OAK_PLANKS, 4),
    },
    SimpleRecipe {
        name: "stick",
        input_a: (item_id::OAK_PLANKS, 2),
        input_b: None,
        output: ItemStack::new(item_id::STICK, 4),
    },
    SimpleRecipe {
        name: "crafting_table",
        input_a: (item_id::OAK_PLANKS, 4),
        input_b: None,
        output: ItemStack::new(item_id::CRAFTING_TABLE, 1),
    },
    SimpleRecipe {
        name: "wooden_pickaxe",
        input_a: (item_id::OAK_PLANKS, 3),
        input_b: Some((item_id::STICK, 2)),
        output: ItemStack::new(item_id::WOODEN_PICKAXE, 1),
    },
    SimpleRecipe {
        name: "wooden_sword",
        input_a: (item_id::OAK_PLANKS, 2),
        input_b: Some((item_id::STICK, 1)),
        output: ItemStack::new(item_id::WOODEN_SWORD, 1),
    },
    SimpleRecipe {
        name: "stone_pickaxe",
        input_a: (item_id::COBBLESTONE, 3),
        input_b: Some((item_id::STICK, 2)),
        output: ItemStack::new(item_id::STONE_PICKAXE, 1),
    },
    SimpleRecipe {
        name: "stone_sword",
        input_a: (item_id::COBBLESTONE, 2),
        input_b: Some((item_id::STICK, 1)),
        output: ItemStack::new(item_id::STONE_SWORD, 1),
    },
    SimpleRecipe {
        name: "iron_ingot",
        input_a: (item_id::RAW_IRON, 1),
        input_b: Some((item_id::COAL, 1)),
        output: ItemStack::new(item_id::IRON_INGOT, 1),
    },
    SimpleRecipe {
        name: "iron_pickaxe",
        input_a: (item_id::IRON_INGOT, 3),
        input_b: Some((item_id::STICK, 2)),
        output: ItemStack::new(item_id::IRON_PICKAXE, 1),
    },
    SimpleRecipe {
        name: "iron_sword",
        input_a: (item_id::IRON_INGOT, 2),
        input_b: Some((item_id::STICK, 1)),
        output: ItemStack::new(item_id::IRON_SWORD, 1),
    },
    SimpleRecipe {
        name: "diamond_pickaxe",
        input_a: (item_id::DIAMOND, 3),
        input_b: Some((item_id::STICK, 2)),
        output: ItemStack::new(item_id::DIAMOND_PICKAXE, 1),
    },
    SimpleRecipe {
        name: "diamond_sword",
        input_a: (item_id::DIAMOND, 2),
        input_b: Some((item_id::STICK, 1)),
        output: ItemStack::new(item_id::DIAMOND_SWORD, 1),
    },
    SimpleRecipe {
        name: "torch",
        input_a: (item_id::COAL, 1),
        input_b: Some((item_id::STICK, 1)),
        output: ItemStack::new(item_id::TORCH, 4),
    },
    SimpleRecipe {
        name: "furnace",
        input_a: (item_id::COBBLESTONE, 8),
        input_b: None,
        output: ItemStack::new(item_id::FURNACE, 1),
    },
    SimpleRecipe {
        name: "chest",
        input_a: (item_id::OAK_PLANKS, 8),
        input_b: None,
        output: ItemStack::new(item_id::CHEST, 1),
    },
    SimpleRecipe {
        name: "cooked_beef",
        input_a: (item_id::BEEF, 1),
        input_b: Some((item_id::COAL, 1)),
        output: ItemStack::new(item_id::COOKED_BEEF, 1),
    },
    SimpleRecipe {
        name: "bow",
        input_a: (item_id::STICK, 3),
        input_b: Some((item_id::STRING, 3)),
        output: ItemStack::new(item_id::BOW, 1),
    },
    SimpleRecipe {
        name: "arrow",
        input_a: (item_id::COBBLESTONE, 1),
        input_b: Some((item_id::STICK, 1)),
        output: ItemStack::new(item_id::ARROW, 4),
    },
    SimpleRecipe {
        name: "ladder",
        input_a: (item_id::STICK, 7),
        input_b: None,
        output: ItemStack::new(item_id::LADDER, 3),
    },
    SimpleRecipe {
        name: "oak_door",
        input_a: (item_id::OAK_PLANKS, 6),
        input_b: None,
        output: ItemStack::new(item_id::OAK_DOOR, 3),
    },
    SimpleRecipe {
        name: "iron_helmet",
        input_a: (item_id::IRON_INGOT, 5),
        input_b: None,
        output: ItemStack::new(item_id::IRON_HELMET, 1),
    },
    SimpleRecipe {
        name: "iron_chestplate",
        input_a: (item_id::IRON_INGOT, 8),
        input_b: None,
        output: ItemStack::new(item_id::IRON_CHESTPLATE, 1),
    },
    SimpleRecipe {
        name: "iron_leggings",
        input_a: (item_id::IRON_INGOT, 7),
        input_b: None,
        output: ItemStack::new(item_id::IRON_LEGGINGS, 1),
    },
    SimpleRecipe {
        name: "iron_boots",
        input_a: (item_id::IRON_INGOT, 4),
        input_b: None,
        output: ItemStack::new(item_id::IRON_BOOTS, 1),
    },
    SimpleRecipe {
        name: "diamond_chestplate",
        input_a: (item_id::DIAMOND, 8),
        input_b: None,
        output: ItemStack::new(item_id::DIAMOND_CHESTPLATE, 1),
    },
    SimpleRecipe {
        name: "diamond_helmet",
        input_a: (item_id::DIAMOND, 5),
        input_b: None,
        output: ItemStack::new(item_id::DIAMOND_HELMET, 1),
    },
    SimpleRecipe {
        name: "gold_ingot",
        input_a: (item_id::RAW_GOLD, 1),
        input_b: Some((item_id::COAL, 1)),
        output: ItemStack::new(item_id::GOLD_INGOT, 1),
    },
    SimpleRecipe {
        name: "golden_apple",
        input_a: (item_id::APPLE, 1),
        input_b: Some((item_id::GOLD_INGOT, 8)),
        output: ItemStack::new(item_id::GOLDEN_APPLE, 1),
    },
    SimpleRecipe {
        name: "shield",
        input_a: (item_id::OAK_PLANKS, 6),
        input_b: Some((item_id::IRON_INGOT, 1)),
        output: ItemStack::new(item_id::SHIELD, 1),
    },
    SimpleRecipe {
        name: "red_bed",
        input_a: (item_id::WHITE_WOOL, 3),
        input_b: Some((item_id::OAK_PLANKS, 3)),
        output: ItemStack::new(item_id::RED_BED, 1),
    },
];

pub fn find_recipe(name: &str) -> Option<&'static SimpleRecipe> {
    let trimmed = name.strip_prefix("minecraft:").unwrap_or(name);
    CORE_RECIPES.iter().find(|r| r.name == trimmed)
}
