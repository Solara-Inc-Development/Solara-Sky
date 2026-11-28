use sky_data::generated_items;
use sky_data::generated_items::id as item_id;

/// Compact 8-byte item stack representation (`item_id = 0` or `count = 0` represents empty).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ItemStack {
    pub item_id: u32,
    pub count: u8,
}

impl ItemStack {
    pub const EMPTY: Self = Self {
        item_id: 0,
        count: 0,
    };

    pub const fn new(item_id: u32, count: u8) -> Self {
        if item_id == 0 || count == 0 {
            Self::EMPTY
        } else {
            Self { item_id, count }
        }
    }

    pub fn from_key(key: &str, count: u8) -> Option<Self> {
        let def = generated_items::by_key(key)?;
        Some(Self::new(def.id, count))
    }

    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.item_id == 0 || self.count == 0
    }

    pub fn key(&self) -> Option<&'static str> {
        if self.is_empty() {
            None
        } else {
            generated_items::by_id(self.item_id).map(|d| d.key)
        }
    }

    pub fn max_stack_size(&self) -> u8 {
        if self.is_empty() {
            return 64;
        }
        match generated_items::by_id(self.item_id).map(|d| d.key) {
            Some(k)
                if k.ends_with("_sword")
                    || k.ends_with("_pickaxe")
                    || k.ends_with("_axe")
                    || k.ends_with("_shovel")
                    || k.ends_with("_hoe")
                    || k.ends_with("_helmet")
                    || k.ends_with("_chestplate")
                    || k.ends_with("_leggings")
                    || k.ends_with("_boots")
                    || k == "minecraft:bow"
                    || k == "minecraft:shield" =>
            {
                1
            }
            Some("minecraft:ender_pearl" | "minecraft:snowball" | "minecraft:egg") => 16,
            _ => 64,
        }
    }

    /// Base melee attack damage when striking an entity with this item in main hand.
    pub const fn melee_attack_damage(&self) -> f32 {
        if self.is_empty() {
            return 4.0;
        }
        match self.item_id {
            item_id::WOODEN_SWORD => 4.5,
            item_id::STONE_SWORD => 5.5,
            item_id::IRON_SWORD => 6.5,
            item_id::DIAMOND_SWORD => 7.5,
            item_id::WOODEN_PICKAXE => 3.0,
            item_id::STONE_PICKAXE => 3.5,
            item_id::IRON_PICKAXE => 4.5,
            item_id::DIAMOND_PICKAXE => 5.5,
            _ => 4.0,
        }
    }

    /// Returns `Some((food_points, saturation_bonus))` if this item is edible in Survival!
    pub const fn food_nutrition(&self) -> Option<(i32, f32)> {
        if self.is_empty() {
            return None;
        }
        match self.item_id {
            item_id::APPLE => Some((4, 2.4)),
            item_id::BREAD => Some((5, 6.0)),
            item_id::PORKCHOP => Some((3, 1.8)),
            item_id::COOKED_PORKCHOP => Some((8, 12.8)),
            item_id::BEEF => Some((3, 1.8)),
            item_id::COOKED_BEEF => Some((8, 12.8)),
            item_id::ROTTEN_FLESH => Some((4, 0.8)),
            _ => None,
        }
    }

    /// Returns `Some((armor_slot_idx, defense_points))` for wearable armor (`0=Boots, 1=Leggings, 2=Chestplate, 3=Helmet`).
    pub const fn armor_slot_and_defense(&self) -> Option<(usize, u8)> {
        if self.is_empty() {
            return None;
        }
        match self.item_id {
            item_id::LEATHER_BOOTS => Some((0, 1)),
            item_id::LEATHER_LEGGINGS => Some((1, 2)),
            item_id::LEATHER_CHESTPLATE => Some((2, 3)),
            item_id::LEATHER_HELMET => Some((3, 1)),
            item_id::IRON_BOOTS => Some((0, 2)),
            item_id::IRON_LEGGINGS => Some((1, 5)),
            item_id::IRON_CHESTPLATE => Some((2, 6)),
            item_id::IRON_HELMET => Some((3, 2)),
            item_id::DIAMOND_BOOTS => Some((0, 3)),
            item_id::DIAMOND_LEGGINGS => Some((1, 6)),
            item_id::DIAMOND_CHESTPLATE => Some((2, 8)),
            item_id::DIAMOND_HELMET => Some((3, 3)),
            _ => None,
        }
    }
}
