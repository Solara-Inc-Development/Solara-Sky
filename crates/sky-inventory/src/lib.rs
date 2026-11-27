//! Player inventories, item stacks, and survival crafting recipes for Solara-Sky.

pub mod item_stack;
pub mod player_inventory;
pub mod recipes;

pub use item_stack::ItemStack;
pub use player_inventory::PlayerInventory;

#[cfg(test)]
mod tests {
    use super::*;
    use sky_data::generated_items::id as item_id;

    #[test]
    fn inventory_merges_partial_stacks_and_crafts_tools() {
        let mut inv = PlayerInventory::new_empty();
        assert_eq!(
            inv.insert(ItemStack::new(item_id::OAK_LOG, 40)),
            ItemStack::EMPTY
        );
        assert_eq!(
            inv.insert(ItemStack::new(item_id::OAK_LOG, 40)),
            ItemStack::EMPTY
        );
        assert_eq!(inv.main[0], ItemStack::new(item_id::OAK_LOG, 64));
        assert_eq!(inv.main[1], ItemStack::new(item_id::OAK_LOG, 16));

        // Craft oak_planks -> stick -> wooden_sword!
        assert_eq!(
            inv.craft_recipe("oak_planks").expect("planks"),
            ItemStack::new(item_id::OAK_PLANKS, 4)
        );
        assert_eq!(
            inv.craft_recipe("oak_planks").expect("planks 2"),
            ItemStack::new(item_id::OAK_PLANKS, 4)
        );
        assert_eq!(
            inv.craft_recipe("stick").expect("sticks"),
            ItemStack::new(item_id::STICK, 4)
        );
        let crafted_sword = inv.craft_recipe("wooden_sword").expect("wooden sword");
        assert_eq!(crafted_sword, ItemStack::new(item_id::WOODEN_SWORD, 1));
        assert_eq!(crafted_sword.max_stack_size(), 1);
    }
}
