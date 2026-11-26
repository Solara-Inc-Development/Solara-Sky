//! Survival player vitals (health, hunger, fall damage), melee combat knockback,
//! and Folia-region-safe mob AI for Solara-Sky.

pub mod combat;
pub mod mob;
pub mod vitals;

pub use combat::compute_melee_knockback;
pub use mob::{MobKind, WorldMob};
pub use vitals::{MAX_FOOD, MAX_HEALTH, PlayerVitals};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fall_damage_and_hurt_immunity_follow_minecraft_rules() {
        let mut v = PlayerVitals::new_full(-50.0);
        // Falling 6 blocks (-50 -> -56) deals 3.0 damage upon landing
        assert_eq!(v.update_movement(-53.0, false, true), 0.0);
        assert_eq!(v.update_movement(-56.0, true, true), 3.0);
        assert_eq!(v.health, 17.0);
        // Immediate second hit inside 10-tick invulnerability deals 0
        assert_eq!(v.apply_damage(4.0), 0.0);
    }

    #[test]
    fn zombie_ai_chases_nearby_player_over_terrain() {
        let mut z = WorldMob::new(99, MobKind::Zombie, [0.0, -59.0, 0.0]);
        let moved = z.step_ai(1, Some([10.0, -59.0, 0.0]), |_, _| -60);
        assert!(moved);
        assert!(z.position[0] > 0.0);
    }
}
