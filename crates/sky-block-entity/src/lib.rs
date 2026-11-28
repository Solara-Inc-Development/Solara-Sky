//! Persistent Chest storage and tick-driven Furnace smelting block entities for Solara-Sky.

pub mod chest;
pub mod furnace;

pub use chest::{CHEST_SLOTS, ChestBlockEntity};
pub use furnace::{COOK_DURATION_TICKS, FurnaceBlockEntity, fuel_burn_ticks, smelt_output_for};

#[cfg(test)]
mod tests {
    use super::*;
    use sky_data::generated_items::id as item_id;
    use sky_inventory::ItemStack;

    #[test]
    fn chest_deposits_and_furnace_smelts_raw_iron_into_ingots() {
        let mut chest = ChestBlockEntity::new();
        assert_eq!(
            chest.deposit(ItemStack::new(item_id::DIAMOND, 12)),
            ItemStack::EMPTY
        );
        assert_eq!(
            chest.withdraw_first(),
            Some(ItemStack::new(item_id::DIAMOND, 12))
        );

        let mut furnace = FurnaceBlockEntity::new();
        assert_eq!(furnace.interact(ItemStack::new(item_id::RAW_IRON, 2)).0, 2);
        assert_eq!(furnace.interact(ItemStack::new(item_id::COAL, 1)).0, 1);
        let mut smelted = false;
        for _ in 0..COOK_DURATION_TICKS {
            smelted |= furnace.tick_smelt();
        }
        assert!(smelted);
        assert_eq!(
            furnace.interact(ItemStack::EMPTY).1,
            Some(ItemStack::new(item_id::IRON_INGOT, 1))
        );
    }
}
