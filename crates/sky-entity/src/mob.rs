use sky_data::generated_entities::id as entity_id;
use sky_data::generated_items::id as item_id;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MobKind {
    Pig,
    Cow,
    Sheep,
    Chicken,
    Zombie,
    Skeleton,
    Spider,
    Creeper,
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
            "chicken" => Some(Self::Chicken),
            "zombie" => Some(Self::Zombie),
            "skeleton" => Some(Self::Skeleton),
            "spider" => Some(Self::Spider),
            "creeper" => Some(Self::Creeper),
            _ => None,
        }
    }

    pub const fn protocol_entity_type(self) -> i32 {
        match self {
            Self::Pig => entity_id::PIG as i32,
            Self::Cow => entity_id::COW as i32,
            Self::Sheep => entity_id::SHEEP as i32,
            Self::Chicken => entity_id::CHICKEN as i32,
            Self::Zombie => entity_id::ZOMBIE as i32,
            Self::Skeleton => entity_id::SKELETON as i32,
            Self::Spider => entity_id::SPIDER as i32,
            Self::Creeper => entity_id::CREEPER as i32,
        }
    }

    pub const fn max_health(self) -> f32 {
        match self {
            Self::Pig => 10.0,
            Self::Cow => 10.0,
            Self::Sheep => 8.0,
            Self::Chicken => 4.0,
            Self::Zombie | Self::Skeleton | Self::Creeper => 20.0,
            Self::Spider => 16.0,
        }
    }

    pub const fn is_hostile(self) -> bool {
        matches!(
            self,
            Self::Zombie | Self::Skeleton | Self::Spider | Self::Creeper
        )
    }

    pub const fn chase_speed(self) -> f64 {
        match self {
            Self::Spider => 0.145,
            Self::Zombie | Self::Creeper => 0.115,
            Self::Skeleton => 0.105,
            _ => 0.065,
        }
    }

    pub const fn melee_damage(self) -> f32 {
        match self {
            Self::Creeper => 6.0,
            Self::Zombie => 3.0,
            Self::Spider | Self::Skeleton => 2.5,
            _ => 0.0,
        }
    }

    /// Survival loot `(item_id, count)` dropped when this mob is defeated.
    pub const fn death_loot(self) -> (u32, u8) {
        match self {
            Self::Pig => (item_id::PORKCHOP, 2),
            Self::Cow => (item_id::BEEF, 2),
            Self::Sheep => (item_id::WHITE_WOOL, 1),
            Self::Chicken => (item_id::FEATHER, 2),
            Self::Zombie => (item_id::ROTTEN_FLESH, 2),
            Self::Skeleton => (item_id::BONE, 2),
            Self::Spider => (item_id::STRING, 2),
            Self::Creeper => (item_id::GUNPOWDER, 2),
        }
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

    /// Steps deterministic wandering / hostile-pursuit AI for one tick (`50 ms`).
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
            if (1.1..22.0).contains(&dist) {
                target_dir = Some((dx / dist, dz / dist, self.kind.chase_speed()));
            }
        }

        if target_dir.is_none() {
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
        if (ground_y as f64 - self.position[1]).abs() > 1.25 {
            return false;
        }
        self.position = [nx, ground_y as f64, nz];
        self.yaw_deg = (vz.atan2(vx).to_degrees() as f32) - 90.0;
        true
    }
}
