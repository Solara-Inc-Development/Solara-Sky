use sky_inventory::ItemStack;

pub const CHEST_SLOTS: usize = 27;

/// 27-slot persistent `minecraft:chest` container stored at a world `(x, y, z)` block position.
#[derive(Debug, Clone)]
pub struct ChestBlockEntity {
    pub slots: [ItemStack; CHEST_SLOTS],
}

impl Default for ChestBlockEntity {
    fn default() -> Self {
        Self::new()
    }
}

impl ChestBlockEntity {
    pub const fn new() -> Self {
        Self {
            slots: [ItemStack::EMPTY; CHEST_SLOTS],
        }
    }

    /// Deposits `stack` into existing matching stacks or empty chest slots; returns leftover `ItemStack`.
    pub fn deposit(&mut self, mut stack: ItemStack) -> ItemStack {
        if stack.is_empty() {
            return ItemStack::EMPTY;
        }
        let max_s = stack.max_stack_size();
        for s in &mut self.slots {
            if s.item_id == stack.item_id && s.count < max_s {
                let take = (max_s - s.count).min(stack.count);
                s.count += take;
                stack.count -= take;
                if stack.count == 0 {
                    return ItemStack::EMPTY;
                }
            }
        }
        for s in &mut self.slots {
            if s.is_empty() {
                let take = max_s.min(stack.count);
                *s = ItemStack::new(stack.item_id, take);
                stack.count -= take;
                if stack.count == 0 {
                    return ItemStack::EMPTY;
                }
            }
        }
        stack
    }

    /// Withdraws the first non-empty stack from the chest (when interacting with an empty hand).
    pub fn withdraw_first(&mut self) -> Option<ItemStack> {
        for s in &mut self.slots {
            if !s.is_empty() {
                let out = *s;
                *s = ItemStack::EMPTY;
                return Some(out);
            }
        }
        None
    }

    /// Drains all non-empty stacks (e.g., to drop items into the world when the chest block breaks).
    pub fn drain_all(&mut self) -> Vec<ItemStack> {
        let mut out = Vec::new();
        for s in &mut self.slots {
            if !s.is_empty() {
                out.push(*s);
                *s = ItemStack::EMPTY;
            }
        }
        out
    }

    pub fn summary(&self) -> String {
        let items: Vec<String> = self
            .slots
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| {
                format!(
                    "{}x {}",
                    s.count,
                    s.key().unwrap_or("item").trim_start_matches("minecraft:")
                )
            })
            .collect();
        if items.is_empty() {
            "empty".to_owned()
        } else {
            items.join(", ")
        }
    }
}
