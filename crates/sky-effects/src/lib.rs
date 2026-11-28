//! Server-side status potion effects (`Speed`, `Strength`, `Regeneration`, `Resistance`,
//! `Night Vision`, `Jump Boost`, `Haste`) for Solara-Sky.

use sky_data::generated_effects::id as effect_id;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveEffect {
    pub effect_id: u32,
    pub amplifier: u8,
    pub duration_ticks: i32,
}

#[derive(Debug, Clone, Default)]
pub struct PlayerEffects {
    active: Vec<ActiveEffect>,
}

pub fn effect_id_from_name(name: &str) -> Option<u32> {
    match name.trim().trim_start_matches("minecraft:") {
        "speed" => Some(effect_id::SPEED),
        "haste" => Some(effect_id::HASTE),
        "strength" => Some(effect_id::STRENGTH),
        "jump_boost" => Some(effect_id::JUMP_BOOST),
        "regeneration" | "regen" => Some(effect_id::REGENERATION),
        "resistance" => Some(effect_id::RESISTANCE),
        "fire_resistance" => Some(effect_id::FIRE_RESISTANCE),
        "night_vision" => Some(effect_id::NIGHT_VISION),
        _ => None,
    }
}

impl PlayerEffects {
    pub const fn new() -> Self {
        Self { active: Vec::new() }
    }

    pub fn active_slice(&self) -> &[ActiveEffect] {
        &self.active
    }

    pub fn apply(&mut self, effect_id: u32, amplifier: u8, duration_ticks: i32) {
        if duration_ticks <= 0 {
            return;
        }
        if let Some(existing) = self.active.iter_mut().find(|e| e.effect_id == effect_id) {
            existing.amplifier = existing.amplifier.max(amplifier);
            existing.duration_ticks = existing.duration_ticks.max(duration_ticks);
        } else {
            self.active.push(ActiveEffect {
                effect_id,
                amplifier,
                duration_ticks,
            });
        }
    }

    pub fn clear(&mut self) -> Vec<u32> {
        self.active.drain(..).map(|e| e.effect_id).collect()
    }

    pub fn has_effect(&self, id: u32) -> bool {
        self.active.iter().any(|e| e.effect_id == id)
    }

    /// Bonus melee damage from `Strength` (`+3.0` per amplifier tier).
    pub fn melee_damage_bonus(&self) -> f32 {
        self.active
            .iter()
            .find(|e| e.effect_id == effect_id::STRENGTH)
            .map_or(0.0, |e| 3.0 * f32::from(e.amplifier + 1))
    }

    /// Incoming damage multiplier from `Resistance` (`0.80` for tier I, `0.60` for tier II, etc.).
    pub fn damage_resistance_multiplier(&self) -> f32 {
        self.active
            .iter()
            .find(|e| e.effect_id == effect_id::RESISTANCE)
            .map_or(1.0, |e| (1.0 - 0.20 * f32::from(e.amplifier + 1)).max(0.20))
    }

    /// Steps active status timers by 1 tick (`50 ms`); returns `(regen_heal_hp, expired_effect_ids)`.
    pub fn tick(&mut self, tick_number: u64) -> (f32, Vec<u32>) {
        let mut regen_hp = 0.0f32;
        if let Some(reg) = self
            .active
            .iter()
            .find(|e| e.effect_id == effect_id::REGENERATION)
        {
            let period = if reg.amplifier >= 1 { 25 } else { 50 };
            if tick_number.is_multiple_of(period) {
                regen_hp = 1.0;
            }
        }

        let mut expired = Vec::new();
        let mut i = 0;
        while i < self.active.len() {
            self.active[i].duration_ticks -= 1;
            if self.active[i].duration_ticks <= 0 {
                expired.push(self.active.swap_remove(i).effect_id);
            } else {
                i += 1;
            }
        }
        (regen_hp, expired)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strength_regeneration_and_expiration_work_accurately() {
        let mut fx = PlayerEffects::new();
        fx.apply(effect_id::STRENGTH, 0, 2);
        fx.apply(effect_id::REGENERATION, 1, 26);
        assert!((fx.melee_damage_bonus() - 3.0).abs() < 1e-5);

        let (h1, exp1) = fx.tick(24);
        assert_eq!(h1, 0.0);
        assert!(exp1.is_empty());

        let (h2, exp2) = fx.tick(25);
        assert_eq!(h2, 1.0);
        assert_eq!(exp2, vec![effect_id::STRENGTH]);
    }
}
