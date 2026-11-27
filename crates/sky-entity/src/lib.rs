//! Survival player vitals (health, hunger, fall damage), melee combat knockback,
//! floating item drops, and Folia-region-safe mob AI for Solara-Sky.

pub mod combat;
pub mod item_drop;
pub mod mob;
pub mod vitals;

pub use combat::compute_melee_knockback;
pub use item_drop::{WorldItemDrop, block_survival_drop};
pub use mob::{MobKind, WorldMob};
pub use vitals::{MAX_FOOD, MAX_HEALTH, PlayerVitals};

#[cfg(test)]
mod tests {
    use super::*;
    use sky_data::generated_items::id as item_id;

    #[test]
    fn fall_damage_and_hurt_immunity_follow_minecraft_rules() {
        let mut v = PlayerVitals::new_full(-50.0);
        // Falling 6 blocks (-50 -> -56) deals 3.0 damage upon landing
        assert_eq!(v.update_movement(-53.0, false, true), 0.0);
        assert_eq!(v.update_movement(-56.0, true, true), 3.0);
        assert_eq!(v.health, 17.0);
        // Immediate second hit inside 10-tick invulnerability deals 0
        assert_eq!(v.apply_damage(4.0), 0.0);

        v.food = 12;
        assert!(v.eat_food(5, 6.0));
        assert_eq!(v.food, 17);
    }

    #[test]
    fn zombie_ai_and_survival_block_drops_work() {
        let mut z = WorldMob::new(99, MobKind::Zombie, [0.0, -59.0, 0.0]);
        let moved = z.step_ai(1, Some([10.0, -59.0, 0.0]), |_, _| -60);
        assert!(moved);
        assert!(z.position[0] > 0.0);
        assert_eq!(
            block_survival_drop("minecraft:deepslate_diamond_ore"),
            Some((item_id::DIAMOND, 1))
        );
        assert_eq!(
            block_survival_drop("minecraft:stone"),
            Some((item_id::COBBLESTONE, 1))
        );
    }
}
