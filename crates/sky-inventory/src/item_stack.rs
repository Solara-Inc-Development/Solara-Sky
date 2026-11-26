use sky_data::generated_items;

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
                    || k.ends_with("_boots") =>
            {
                1
            }
            Some("minecraft:ender_pearl" | "minecraft:snowball" | "minecraft:egg") => 16,
            _ => 64,
        }
    }
}
