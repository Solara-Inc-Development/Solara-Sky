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
    fn inventory_merges_partial_stacks_and_respects_unstackable_tools() {
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

        let sword = ItemStack::new(item_id::DIAMOND_SWORD, 1);
        assert_eq!(sword.max_stack_size(), 1);
        assert!(recipes::find_recipe("crafting_table").is_some());
    }
}
