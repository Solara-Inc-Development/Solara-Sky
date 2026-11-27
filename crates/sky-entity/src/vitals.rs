pub const MAX_HEALTH: f32 = 20.0;
pub const MAX_FOOD: i32 = 20;

/// Survival health, hunger, saturation, hurt invulnerability, and fall-damage state.
#[derive(Debug, Clone, Copy)]
pub struct PlayerVitals {
    pub health: f32,
    pub food: i32,
    pub saturation: f32,
    pub fall_distance: f32,
    pub last_y: f64,
    pub hurt_invuln_ticks: u8,
}

impl Default for PlayerVitals {
    fn default() -> Self {
        Self::new_full(0.0)
    }
}

impl PlayerVitals {
    pub fn new_full(spawn_y: f64) -> Self {
        Self {
            health: MAX_HEALTH,
            food: MAX_FOOD,
            saturation: 5.0,
            fall_distance: 0.0,
            last_y: spawn_y,
            hurt_invuln_ticks: 0,
        }
    }

    pub fn is_dead(&self) -> bool {
        self.health <= 0.0
    }

    pub fn respawn_reset(&mut self, spawn_y: f64) {
        *self = Self::new_full(spawn_y);
    }

    pub fn heal_full(&mut self) {
        self.health = MAX_HEALTH;
    }

    pub fn feed_full(&mut self) {
        self.food = MAX_FOOD;
        self.saturation = 5.0;
    }

    /// Consumes food in Survival if hungry (`food < 20`), restoring hunger & saturation.
    pub fn eat_food(&mut self, food_points: i32, saturation_bonus: f32) -> bool {
        if self.is_dead() || self.food >= MAX_FOOD {
            return false;
        }
        self.food = (self.food + food_points).clamp(0, MAX_FOOD);
        self.saturation = (self.saturation + saturation_bonus).min(self.food as f32);
        true
    }

    /// Applies incoming damage if outside hurt-invulnerability frames; returns actual damage dealt.
    pub fn apply_damage(&mut self, amount: f32) -> f32 {
        if amount <= 0.0 || self.hurt_invuln_ticks > 0 || self.is_dead() {
            return 0.0;
        }
        let dealt = amount.min(self.health);
        self.health = (self.health - amount).max(0.0);
        self.hurt_invuln_ticks = 10; // 0.5s (10 ticks) classic Minecraft hurt immunity
        dealt
    }

    /// Updates fall distance on player movement and returns fall damage (`> 0.0`) when landing!
    pub fn update_movement(&mut self, new_y: f64, on_ground: bool, vulnerable: bool) -> f32 {
        let dy = new_y - self.last_y;
        self.last_y = new_y;

        if !vulnerable {
            self.fall_distance = 0.0;
            return 0.0;
        }

        if dy < 0.0 {
            self.fall_distance += (-dy) as f32;
        }

        if on_ground {
            let dist = self.fall_distance;
            self.fall_distance = 0.0;
            if dist > 3.25 {
                let raw_dmg = (dist - 3.0).ceil();
                self.apply_damage(raw_dmg)
            } else {
                0.0
            }
        } else {
            0.0
        }
    }

    /// Advances invulnerability timer and natural survival health regeneration (`food >= 18`).
    /// Returns `true` if health/food changed and should be synced via `SetHealth`.
    pub fn tick_natural(&mut self, tick_number: u64) -> bool {
        if self.hurt_invuln_ticks > 0 {
            self.hurt_invuln_ticks -= 1;
        }
        if !self.is_dead()
            && self.food >= 18
            && self.health < MAX_HEALTH
            && tick_number.is_multiple_of(80)
        {
            self.health = (self.health + 1.0).min(MAX_HEALTH);
            if self.saturation > 0.0 {
                self.saturation = (self.saturation - 1.0).max(0.0);
            } else {
                self.food = (self.food - 1).max(0);
            }
            return true;
        }
        false
    }
}
