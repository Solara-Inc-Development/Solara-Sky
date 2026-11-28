use sky_data::generated_items::id as item_id;
use sky_inventory::ItemStack;

/// Tick-driven `minecraft:furnace` block entity smelting ores, sand, cobble, and raw meats.
#[derive(Debug, Clone, Default)]
pub struct FurnaceBlockEntity {
    pub input: ItemStack,
    pub fuel: ItemStack,
    pub output: ItemStack,
    pub burn_ticks_left: u16,
    pub cook_progress_ticks: u16,
}

pub const COOK_DURATION_TICKS: u16 = 60; // Fast 3s responsive Survival smelting

/// Resolves the smelted output item ID for a valid raw input item ID.
pub const fn smelt_output_for(input_item_id: u32) -> Option<u32> {
    match input_item_id {
        item_id::RAW_IRON => Some(item_id::IRON_INGOT),
        item_id::RAW_GOLD => Some(item_id::GOLD_INGOT),
        item_id::SAND => Some(item_id::GLASS),
        item_id::COBBLESTONE => Some(item_id::STONE),
        item_id::BEEF => Some(item_id::COOKED_BEEF),
        item_id::PORKCHOP => Some(item_id::COOKED_PORKCHOP),
        _ => None,
    }
}

/// Fuel burn duration in ticks (`50 ms` per tick).
pub const fn fuel_burn_ticks(fuel_item_id: u32) -> Option<u16> {
    match fuel_item_id {
        item_id::COAL => Some(600),
        item_id::OAK_LOG => Some(150),
        item_id::OAK_PLANKS => Some(120),
        item_id::STICK => Some(60),
        _ => None,
    }
}

impl FurnaceBlockEntity {
    pub const fn new() -> Self {
        Self {
            input: ItemStack::EMPTY,
            fuel: ItemStack::EMPTY,
            output: ItemStack::EMPTY,
            burn_ticks_left: 0,
            cook_progress_ticks: 0,
        }
    }

    /// Steps the furnace by 1 server tick (`50 ms`). Returns `true` when an item finishes smelting.
    pub fn tick_smelt(&mut self) -> bool {
        if self.burn_ticks_left > 0 {
            self.burn_ticks_left -= 1;
        }

        let Some(target_out) = (!self.input.is_empty())
            .then(|| smelt_output_for(self.input.item_id))
            .flatten()
        else {
            self.cook_progress_ticks = 0;
            return false;
        };

        let can_fit_output =
            self.output.is_empty() || (self.output.item_id == target_out && self.output.count < 64);
        if !can_fit_output {
            self.cook_progress_ticks = 0;
            return false;
        }

        if self.burn_ticks_left == 0
            && !self.fuel.is_empty()
            && let Some(burn) = fuel_burn_ticks(self.fuel.item_id)
        {
            self.fuel.count -= 1;
            if self.fuel.count == 0 {
                self.fuel = ItemStack::EMPTY;
            }
            self.burn_ticks_left = burn;
        }

        if self.burn_ticks_left == 0 {
            self.cook_progress_ticks = self.cook_progress_ticks.saturating_sub(1);
            return false;
        }

        self.cook_progress_ticks += 1;
        if self.cook_progress_ticks >= COOK_DURATION_TICKS {
            self.cook_progress_ticks = 0;
            self.input.count -= 1;
            if self.input.count == 0 {
                self.input = ItemStack::EMPTY;
            }
            if self.output.is_empty() {
                self.output = ItemStack::new(target_out, 1);
            } else {
                self.output.count += 1;
            }
            return true;
        }
        false
    }

    /// Collects finished output if ready, or inserts fuel/smeltable input from `held`.
    /// Returns `(taken_from_held, collected_output)`.
    pub fn interact(&mut self, held: ItemStack) -> (u8, Option<ItemStack>) {
        if !self.output.is_empty() {
            let ready = self.output;
            self.output = ItemStack::EMPTY;
            return (0, Some(ready));
        }
        if held.is_empty() {
            return (0, None);
        }
        if smelt_output_for(held.item_id).is_some()
            && (self.input.is_empty()
                || (self.input.item_id == held.item_id && self.input.count < 64))
        {
            let space = if self.input.is_empty() {
                64
            } else {
                64 - self.input.count
            };
            let move_cnt = space.min(held.count);
            if self.input.is_empty() {
                self.input = ItemStack::new(held.item_id, move_cnt);
            } else {
                self.input.count += move_cnt;
            }
            return (move_cnt, None);
        }
        if fuel_burn_ticks(held.item_id).is_some()
            && (self.fuel.is_empty() || (self.fuel.item_id == held.item_id && self.fuel.count < 64))
        {
            let space = if self.fuel.is_empty() {
                64
            } else {
                64 - self.fuel.count
            };
            let move_cnt = space.min(held.count);
            if self.fuel.is_empty() {
                self.fuel = ItemStack::new(held.item_id, move_cnt);
            } else {
                self.fuel.count += move_cnt;
            }
            return (move_cnt, None);
        }
        (0, None)
    }

    pub fn drain_all(&mut self) -> Vec<ItemStack> {
        let mut out = Vec::new();
        for s in [&mut self.input, &mut self.fuel, &mut self.output] {
            if !s.is_empty() {
                out.push(*s);
                *s = ItemStack::EMPTY;
            }
        }
        out
    }
}
