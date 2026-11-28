//! Ballistic 3D projectile simulation (`minecraft:arrow`) for Solara-Sky.

use uuid::Uuid;

/// Active 3D arrow projectile flying through the Overworld (`minecraft:arrow`).
#[derive(Debug, Clone)]
pub struct WorldArrow {
    pub entity_id: i32,
    pub uuid: Uuid,
    pub shooter_entity_id: i32,
    pub position: [f64; 3],
    pub prev_position: [f64; 3],
    pub velocity: [f64; 3],
    pub yaw_deg: f32,
    pub pitch_deg: f32,
    pub ticks_alive: u16,
}

impl WorldArrow {
    pub const PROTOCOL_ENTITY_TYPE: i32 = sky_data::generated_entities::id::ARROW as i32;
    pub const BASE_DAMAGE: f32 = 8.0;
    pub const MAX_LIFETIME_TICKS: u16 = 120;

    /// Spawns a full-draw bow arrow from `eye_pos` using standard Minecraft `(yaw_deg, pitch_deg)`.
    pub fn from_aim(
        entity_id: i32,
        shooter_entity_id: i32,
        eye_pos: [f64; 3],
        yaw_deg: f32,
        pitch_deg: f32,
    ) -> Self {
        let yaw_rad = f64::from(yaw_deg).to_radians();
        let pitch_rad = f64::from(pitch_deg).to_radians();
        let cp = pitch_rad.cos();
        let dir = [-yaw_rad.sin() * cp, -pitch_rad.sin(), yaw_rad.cos() * cp];
        let speed = 1.45; // ~29 blocks/sec (50 ms tick)
        let start = [
            eye_pos[0] + dir[0] * 0.45,
            eye_pos[1] - 0.10 + dir[1] * 0.45,
            eye_pos[2] + dir[2] * 0.45,
        ];
        Self {
            entity_id,
            uuid: Uuid::new_v4(),
            shooter_entity_id,
            position: start,
            prev_position: start,
            velocity: [dir[0] * speed, dir[1] * speed, dir[2] * speed],
            yaw_deg,
            pitch_deg,
            ticks_alive: 0,
        }
    }

    /// Advances ballistic trajectory by 1 server tick (`50 ms`: velocity + drag + gravity).
    pub fn step_flight(&mut self) {
        self.prev_position = self.position;
        self.position[0] += self.velocity[0];
        self.position[1] += self.velocity[1];
        self.position[2] += self.velocity[2];
        self.velocity[0] *= 0.99;
        self.velocity[1] = self.velocity[1] * 0.99 - 0.05;
        self.velocity[2] *= 0.99;
        let horiz = (self.velocity[0] * self.velocity[0] + self.velocity[2] * self.velocity[2])
            .sqrt()
            .max(1e-6);
        self.pitch_deg = (-self.velocity[1].atan2(horiz).to_degrees()) as f32;
        self.ticks_alive = self.ticks_alive.saturating_add(1);
    }

    pub const fn is_expired(&self) -> bool {
        self.ticks_alive >= Self::MAX_LIFETIME_TICKS
    }

    /// Checks swept segment `[prev_position -> position]` against an expanded entity AABB.
    pub fn intersects_target(&self, feet_pos: [f64; 3], half_w: f64, height: f64) -> bool {
        let pad = 0.25;
        let min_b = [
            feet_pos[0] - half_w - pad,
            feet_pos[1] - pad,
            feet_pos[2] - half_w - pad,
        ];
        let max_b = [
            feet_pos[0] + half_w + pad,
            feet_pos[1] + height + pad,
            feet_pos[2] + half_w + pad,
        ];
        let seg = [
            self.position[0] - self.prev_position[0],
            self.position[1] - self.prev_position[1],
            self.position[2] - self.prev_position[2],
        ];
        let mut tmin = 0.0f64;
        let mut tmax = 1.0f64;
        for axis in 0..3 {
            if seg[axis].abs() < 1e-9 {
                if self.prev_position[axis] < min_b[axis] || self.prev_position[axis] > max_b[axis]
                {
                    return false;
                }
            } else {
                let inv = 1.0 / seg[axis];
                let mut t1 = (min_b[axis] - self.prev_position[axis]) * inv;
                let mut t2 = (max_b[axis] - self.prev_position[axis]) * inv;
                if t1 > t2 {
                    std::mem::swap(&mut t1, &mut t2);
                }
                tmin = tmin.max(t1);
                tmax = tmax.min(t2);
                if tmin > tmax {
                    return false;
                }
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrow_flies_forward_and_detects_target_intersection() {
        // yaw = 0 in Minecraft faces +Z
        let mut arrow = WorldArrow::from_aim(500, 1, [0.0, 1.62, 0.0], 0.0, 0.0);
        arrow.step_flight();
        assert!(arrow.position[2] > 1.5);
        assert!(arrow.intersects_target([0.0, 0.0, 1.8], 0.4, 1.95));
        assert!(!arrow.intersects_target([10.0, 0.0, 1.8], 0.4, 1.95));
    }
}
