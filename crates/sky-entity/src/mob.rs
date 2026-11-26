use sky_data::generated_entities::id as entity_id;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MobKind {
    Pig,
    Cow,
    Sheep,
    Zombie,
}

impl MobKind {
    pub fn from_name(name: &str) -> Option<Self> {
        match name
            .trim()
            .strip_prefix("minecraft:")
            .unwrap_or(name)
            .to_ascii_lowercase()
            .as_str()
        {
            "pig" => Some(Self::Pig),
            "cow" => Some(Self::Cow),
            "sheep" => Some(Self::Sheep),
            "zombie" => Some(Self::Zombie),
            _ => None,
        }
    }

    pub const fn protocol_entity_type(self) -> i32 {
        match self {
            Self::Pig => entity_id::PIG as i32,
            Self::Cow => entity_id::COW as i32,
            Self::Sheep => entity_id::SHEEP as i32,
            Self::Zombie => entity_id::ZOMBIE as i32,
        }
    }

    pub const fn max_health(self) -> f32 {
        match self {
            Self::Pig => 10.0,
            Self::Cow => 10.0,
            Self::Sheep => 8.0,
            Self::Zombie => 20.0,
        }
    }

    pub const fn is_hostile(self) -> bool {
        matches!(self, Self::Zombie)
    }
}

/// Active server-side mob ticked inside its Folia spatial region.
#[derive(Debug, Clone)]
pub struct WorldMob {
    pub entity_id: i32,
    pub uuid: Uuid,
    pub kind: MobKind,
    pub position: [f64; 3],
    pub yaw_deg: f32,
    pub health: f32,
}

impl WorldMob {
    pub fn new(entity_id: i32, kind: MobKind, position: [f64; 3]) -> Self {
        Self {
            entity_id,
            uuid: Uuid::new_v4(),
            kind,
            position,
            yaw_deg: 0.0,
            health: kind.max_health(),
        }
    }

    /// Steps simple deterministic wandering / hostile-pursuit AI for one tick (`50 ms`).
    /// `surface_y_fn` returns the terrain top block Y at `(wx, wz)` so mobs walk naturally over hills!
    pub fn step_ai(
        &mut self,
        tick_number: u64,
        nearest_survival_player: Option<[f64; 3]>,
        surface_y_fn: impl Fn(i32, i32) -> i32,
    ) -> bool {
        let mut target_dir: Option<(f64, f64, f64)> = None;

        if self.kind.is_hostile()
            && let Some(ppos) = nearest_survival_player
        {
            let dx = ppos[0] - self.position[0];
            let dz = ppos[2] - self.position[2];
            let dist = (dx * dx + dz * dz).sqrt();
            if (1.1..20.0).contains(&dist) {
                target_dir = Some((dx / dist, dz / dist, 0.115)); // ~2.3 m/s zombie chase
            }
        }

        if target_dir.is_none() {
            // Peaceful deterministic wander phase
            let phase = (tick_number.wrapping_add(self.entity_id as u64 * 37)) % 120;
            if phase < 50 {
                let angle_rad = (((tick_number / 120)
                    .wrapping_mul(2654435761)
                    .wrapping_add(self.entity_id as u64 * 97))
                    % 360) as f64
                    * std::f64::consts::PI
                    / 180.0;
                target_dir = Some((angle_rad.cos(), angle_rad.sin(), 0.065));
            }
        }

        let Some((vx, vz, speed)) = target_dir else {
            return false;
        };

        let nx = self.position[0] + vx * speed;
        let nz = self.position[2] + vz * speed;
        let ground_y = surface_y_fn(nx.floor() as i32, nz.floor() as i32) + 1;
        // Avoid climbing sheer cliffs > 1 block high
        if (ground_y as f64 - self.position[1]).abs() > 1.25 {
            return false;
        }
        self.position = [nx, ground_y as f64, nz];
        self.yaw_deg = (vz.atan2(vx).to_degrees() as f32) - 90.0;
        true
    }
}
