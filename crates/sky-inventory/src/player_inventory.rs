use crate::item_stack::ItemStack;
use crate::recipes::find_recipe;
use sky_data::generated_items::id as item_id;

pub const HOTBAR_SLOTS: usize = 9;
pub const MAIN_INVENTORY_SLOTS: usize = 36;
pub const ARMOR_SLOTS: usize = 4;

/// 36-slot main player inventory (`0..9` hotbar, `9..36` storage) + armor + offhand.
#[derive(Debug, Clone)]
pub struct PlayerInventory {
    pub main: [ItemStack; MAIN_INVENTORY_SLOTS],
    pub armor: [ItemStack; ARMOR_SLOTS],
    pub offhand: ItemStack,
    pub selected_hotbar: usize,
}

impl Default for PlayerInventory {
    fn default() -> Self {
        Self::new_starter_kit()
    }
}

impl PlayerInventory {
    pub fn new_empty() -> Self {
        Self {
            main: [ItemStack::EMPTY; MAIN_INVENTORY_SLOTS],
            armor: [ItemStack::EMPTY; ARMOR_SLOTS],
            offhand: ItemStack::EMPTY,
            selected_hotbar: 0,
        }
    }

    pub fn new_starter_kit() -> Self {
        let mut inv = Self::new_empty();
        let starter_hotbar = [
            ItemStack::new(item_id::STONE, 64),
            ItemStack::new(item_id::COBBLESTONE, 64),
            ItemStack::new(item_id::OAK_PLANKS, 64),
            ItemStack::new(item_id::OAK_LOG, 64),
            ItemStack::new(item_id::DIRT, 64),
            ItemStack::new(item_id::GRASS_BLOCK, 64),
            ItemStack::new(item_id::SAND, 64),
            ItemStack::new(item_id::GLASS, 64),
            ItemStack::new(item_id::GLOWSTONE, 64),
        ];
        inv.main[..HOTBAR_SLOTS].copy_from_slice(&starter_hotbar);
        // Survival adventure supplies in backpack slots 9..15
        inv.main[9] = ItemStack::new(item_id::WOODEN_PICKAXE, 1);
        inv.main[10] = ItemStack::new(item_id::WOODEN_SWORD, 1);
        inv.main[11] = ItemStack::new(item_id::BREAD, 8);
        inv.main[12] = ItemStack::new(item_id::TORCH, 16);
        inv.main[13] = ItemStack::new(item_id::BOW, 1);
        inv.main[14] = ItemStack::new(item_id::ARROW, 16);
        inv
    }

    pub fn held_stack(&self) -> ItemStack {
        self.main[self.selected_hotbar.min(HOTBAR_SLOTS - 1)]
    }

    pub fn count_item(&self, target_item: u32) -> u32 {
        self.main
            .iter()
            .filter(|s| !s.is_empty() && s.item_id == target_item)
            .map(|s| u32::from(s.count))
            .sum()
    }

    /// Consumes `amount` items from `slot_idx`, returning the updated `ItemStack` in that slot.
    pub fn consume_slot(&mut self, slot_idx: usize, amount: u8) -> Option<ItemStack> {
        let slot = self.main.get_mut(slot_idx)?;
        if slot.is_empty() || slot.count < amount {
            return None;
        }
        slot.count -= amount;
        if slot.count == 0 {
            *slot = ItemStack::EMPTY;
        }
        Some(*slot)
    }

    /// Removes `amount` total items of `target_item` across `main` slots. Returns `true` on success.
    pub fn consume_item(&mut self, target_item: u32, mut amount: u8) -> bool {
        if self.count_item(target_item) < u32::from(amount) {
            return false;
        }
        for slot in &mut self.main {
            if amount == 0 {
                break;
            }
            if !slot.is_empty() && slot.item_id == target_item {
                let take = slot.count.min(amount);
                slot.count -= take;
                amount -= take;
                if slot.count == 0 {
                    *slot = ItemStack::EMPTY;
                }
            }
        }
        true
    }

    /// Crafts a named `SimpleRecipe` by consuming ingredients from `main` and inserting the output!
    pub fn craft_recipe(&mut self, recipe_name: &str) -> Result<ItemStack, &'static str> {
        let Some(recipe) = find_recipe(recipe_name) else {
            return Err("Unknown recipe (try /recipes).");
        };
        if self.count_item(recipe.input_a.0) < u32::from(recipe.input_a.1) {
            return Err("Not enough primary crafting materials in inventory.");
        }
        if let Some((item_b, cnt_b)) = recipe.input_b
            && self.count_item(item_b) < u32::from(cnt_b)
        {
            return Err("Not enough secondary crafting materials (e.g. sticks/coal).");
        }
        let _ = self.consume_item(recipe.input_a.0, recipe.input_a.1);
        if let Some((item_b, cnt_b)) = recipe.input_b {
            let _ = self.consume_item(item_b, cnt_b);
        }
        let leftover = self.insert(recipe.output);
        if !leftover.is_empty() {
            return Err("Crafted item partially overflowed full inventory.");
        }
        Ok(recipe.output)
    }

    /// Inserts as much of `stack` as fits into existing partial stacks then empty slots.
    /// Returns the leftover `ItemStack` (`EMPTY` when fully inserted).
    pub fn insert(&mut self, mut stack: ItemStack) -> ItemStack {
        if stack.is_empty() {
            return ItemStack::EMPTY;
        }
        let max_stack = stack.max_stack_size();

        // 1. Merge into matching partial stacks
        for slot in &mut self.main {
            if slot.item_id == stack.item_id && slot.count < max_stack {
                let space = max_stack - slot.count;
                let moved = space.min(stack.count);
                slot.count += moved;
                stack.count -= moved;
                if stack.count == 0 {
                    return ItemStack::EMPTY;
                }
            }
        }

        // 2. Fill empty slots
        for slot in &mut self.main {
            if slot.is_empty() {
                let moved = max_stack.min(stack.count);
                *slot = ItemStack::new(stack.item_id, moved);
                stack.count -= moved;
                if stack.count == 0 {
                    return ItemStack::EMPTY;
                }
            }
        }

        stack
    }

    pub fn clear(&mut self) {
        self.main.fill(ItemStack::EMPTY);
        self.armor.fill(ItemStack::EMPTY);
        self.offhand = ItemStack::EMPTY;
    }

    /// Equips a wearable armor item into `self.armor[0..4]`, returning `(armor_slot_idx, previous_piece)`.
    pub fn equip_armor(&mut self, piece: ItemStack) -> Option<(usize, ItemStack)> {
        let (armor_idx, _) = piece.armor_slot_and_defense()?;
        let prev = self.armor[armor_idx];
        self.armor[armor_idx] = ItemStack::new(piece.item_id, 1);
        Some((armor_idx, prev))
    }

    /// Total equipped armor defense points (`0..=20`).
    pub fn total_armor_points(&self) -> u8 {
        self.armor
            .iter()
            .filter_map(|s| s.armor_slot_and_defense().map(|(_, pts)| pts))
            .sum::<u8>()
            .min(20)
    }
}
