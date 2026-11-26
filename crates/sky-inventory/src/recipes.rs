use crate::item_stack::ItemStack;
use sky_data::generated_items::id as item_id;

#[derive(Debug, Clone, Copy)]
pub struct SimpleRecipe {
    pub name: &'static str,
    pub input_item: u32,
    pub input_count: u8,
    pub output: ItemStack,
}

/// Built-in survival crafting recipes usable via crafting stations and `/craft`.
pub static CORE_RECIPES: &[SimpleRecipe] = &[
    SimpleRecipe {
        name: "oak_planks",
        input_item: item_id::OAK_LOG,
        input_count: 1,
        output: ItemStack::new(item_id::OAK_PLANKS, 4),
    },
    SimpleRecipe {
        name: "stick",
        input_item: item_id::OAK_PLANKS,
        input_count: 2,
        output: ItemStack::new(item_id::STICK, 4),
    },
    SimpleRecipe {
        name: "crafting_table",
        input_item: item_id::OAK_PLANKS,
        input_count: 4,
        output: ItemStack::new(item_id::CRAFTING_TABLE, 1),
    },
    SimpleRecipe {
        name: "furnace",
        input_item: item_id::COBBLESTONE,
        input_count: 8,
        output: ItemStack::new(item_id::FURNACE, 1),
    },
    SimpleRecipe {
        name: "chest",
        input_item: item_id::OAK_PLANKS,
        input_count: 8,
        output: ItemStack::new(item_id::CHEST, 1),
    },
];

pub fn find_recipe(name: &str) -> Option<&'static SimpleRecipe> {
    let trimmed = name.strip_prefix("minecraft:").unwrap_or(name);
    CORE_RECIPES.iter().find(|r| r.name == trimmed)
}
