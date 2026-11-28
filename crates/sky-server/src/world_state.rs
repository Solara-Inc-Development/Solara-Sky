use anyhow::{Context, Result};
use bytes::Bytes;
use rayon::prelude::*;
use sky_block_entity::{ChestBlockEntity, FurnaceBlockEntity};
use sky_entity::{MobKind, PlayerVitals, WorldItemDrop, WorldMob, compute_melee_knockback};
use sky_inventory::{ItemStack, PlayerInventory};
use sky_projectile::WorldArrow;
use sky_protocol::config_play::GameMode;
use sky_protocol::packets::ProfileProperty;
use sky_world::anvil::{AnvilRegion, region_coords, region_file_path};
use sky_world::terrain::{TerrainGenerator, worker_threads};
use sky_world::{ChunkPos, WorldChunk, generated_blocks};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;
use tracing::info;
use uuid::Uuid;

pub const WORLD_SEED: u64 = 0x501A_2A26_3000_0777;

#[derive(Debug, Clone)]
pub enum BroadcastEvent {
    Packet {
        exclude_entity_id: Option<i32>,
        packet_id: i32,
        payload: Bytes,
    },
    DirectPlayer {
        target_entity_id: i32,
        packet_id: i32,
        payload: Bytes,
    },
    Kick {
        target_username_lower: String,
        reason: String,
    },
}

#[derive(Debug, Clone)]
pub struct ConnectedPlayer {
    pub entity_id: i32,
    pub username: String,
    pub uuid: Uuid,
    pub properties: Vec<ProfileProperty>,
    pub position: [f64; 3],
    pub yaw_deg: f32,
    pub pitch_deg: f32,
    pub game_mode: GameMode,
    pub vitals: PlayerVitals,
    pub inventory: PlayerInventory,
}

#[derive(Debug, Default)]
pub struct WorldTickOutcome {
    pub moved_mobs: Vec<WorldMob>,
    pub moved_arrows: Vec<WorldArrow>,
    pub arrow_hit_mobs: Vec<(WorldMob, f32, bool)>,
    pub xp_updates: Vec<(i32, PlayerVitals)>,
    pub healed_players: Vec<(i32, PlayerVitals)>,
    pub zombie_hits: Vec<(ConnectedPlayer, f32)>,
    pub picked_up_items: Vec<(i32, i32, u8, PlayerInventory)>,
    pub despawned_items: Vec<i32>,
}

/// Thread-safe multi-core procedural Overworld chunk store, zero-copy `Bytes` packet cache,
/// survival player roster, world mobs, 3D arrows, floating item drops, Overworld clock, and Anvil persistence.
pub struct LiveWorld {
    region_dir: PathBuf,
    generator: TerrainGenerator,
    chunks: RwLock<HashMap<ChunkPos, WorldChunk>>,
    chunk_packet_cache: RwLock<HashMap<ChunkPos, Bytes>>,
    dirty_chunks: RwLock<HashSet<ChunkPos>>,
    players: RwLock<HashMap<i32, ConnectedPlayer>>,
    mobs: RwLock<HashMap<i32, WorldMob>>,
    arrows: RwLock<HashMap<i32, WorldArrow>>,
    item_drops: RwLock<HashMap<i32, WorldItemDrop>>,
    chests: RwLock<HashMap<(i32, i32, i32), ChestBlockEntity>>,
    furnaces: RwLock<HashMap<(i32, i32, i32), FurnaceBlockEntity>>,
    raining: AtomicBool,
    game_time: AtomicI64,
    day_time: AtomicI64,
    broadcast_tx: broadcast::Sender<BroadcastEvent>,
}

impl LiveWorld {
    pub fn load_or_generate(region_dir: PathBuf, spawn_radius: i32) -> Result<Self> {
        std::fs::create_dir_all(&region_dir)
            .with_context(|| format!("create region dir {}", region_dir.display()))?;

        let generator = TerrainGenerator::new(WORLD_SEED);
        let mut region_cache: HashMap<(i32, i32), AnvilRegion> = HashMap::new();
        let mut disk_chunks: Vec<WorldChunk> = Vec::new();
        let mut missing_coords: Vec<ChunkPos> = Vec::new();

        for cz in -spawn_radius..=spawn_radius {
            for cx in -spawn_radius..=spawn_radius {
                let pos = ChunkPos { x: cx, z: cz };
                let rcoords = region_coords(pos);
                if let std::collections::hash_map::Entry::Vacant(e) = region_cache.entry(rcoords) {
                    let path = region_file_path(&region_dir, rcoords.0, rcoords.1);
                    let reg = AnvilRegion::load_or_create(&path)?;
                    e.insert(reg);
                }
                let reg = &region_cache[&rcoords];
                if let Some(saved) = reg.get_chunk(pos)? {
                    disk_chunks.push(saved);
                } else {
                    missing_coords.push(pos);
                }
            }
        }

        let loaded_from_disk = disk_chunks.len();
        let generated_count = missing_coords.len();

        let encoded_disk: Vec<(ChunkPos, WorldChunk, Vec<u8>)> = disk_chunks
            .into_par_iter()
            .map(|chunk| {
                let pos = chunk.pos;
                let pkt = chunk.encode_level_chunk_with_light(false);
                (pos, chunk, pkt)
            })
            .collect();

        let encoded_generated = generator.generate_and_encode_batch_parallel(&missing_coords);

        let total_cap = encoded_disk.len() + encoded_generated.len();
        let mut chunks = HashMap::with_capacity(total_cap);
        let mut packet_cache = HashMap::with_capacity(total_cap);

        for (pos, chunk, pkt) in encoded_disk.into_iter().chain(encoded_generated) {
            packet_cache.insert(pos, Bytes::from(pkt));
            chunks.insert(pos, chunk);
        }

        // Seed 8 diverse Overworld mobs around the spawn meadows (entity IDs 1..=8)
        let mut initial_mobs = HashMap::new();
        let starter_spawns = [
            (1, MobKind::Pig, 16, 8),
            (2, MobKind::Cow, -4, 14),
            (3, MobKind::Sheep, 12, -6),
            (4, MobKind::Chicken, -8, -4),
            (5, MobKind::Zombie, 24, 20),
            (6, MobKind::Skeleton, -20, 22),
            (7, MobKind::Spider, 26, -16),
            (8, MobKind::Creeper, -22, -18),
        ];
        for (id, kind, wx, wz) in starter_spawns {
            let sy = (generator.surface_height(wx, wz) + 1) as f64;
            initial_mobs.insert(
                id,
                WorldMob::new(id, kind, [wx as f64 + 0.5, sy, wz as f64 + 0.5]),
            );
        }

        info!(
            worker_threads = worker_threads(),
            generated_chunks = generated_count,
            loaded_from_disk,
            spawned_mobs = initial_mobs.len(),
            "multi-core spawn world initialization complete"
        );

        let (broadcast_tx, _) = broadcast::channel(2048);
        Ok(Self {
            region_dir,
            generator,
            chunks: RwLock::new(chunks),
            chunk_packet_cache: RwLock::new(packet_cache),
            dirty_chunks: RwLock::new(HashSet::new()),
            players: RwLock::new(HashMap::new()),
            mobs: RwLock::new(initial_mobs),
            arrows: RwLock::new(HashMap::new()),
            item_drops: RwLock::new(HashMap::new()),
            chests: RwLock::new(HashMap::new()),
            furnaces: RwLock::new(HashMap::new()),
            raining: AtomicBool::new(false),
            game_time: AtomicI64::new(0),
            day_time: AtomicI64::new(1000),
            broadcast_tx,
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<BroadcastEvent> {
        self.broadcast_tx.subscribe()
    }

    pub fn broadcast_all(&self, packet_id: i32, payload: impl Into<Bytes>) {
        let _ = self.broadcast_tx.send(BroadcastEvent::Packet {
            exclude_entity_id: None,
            packet_id,
            payload: payload.into(),
        });
    }

    pub fn broadcast_others(
        &self,
        sender_entity_id: i32,
        packet_id: i32,
        payload: impl Into<Bytes>,
    ) {
        let _ = self.broadcast_tx.send(BroadcastEvent::Packet {
            exclude_entity_id: Some(sender_entity_id),
            packet_id,
            payload: payload.into(),
        });
    }

    pub fn send_direct(&self, target_entity_id: i32, packet_id: i32, payload: impl Into<Bytes>) {
        let _ = self.broadcast_tx.send(BroadcastEvent::DirectPlayer {
            target_entity_id,
            packet_id,
            payload: payload.into(),
        });
    }

    pub fn kick_player(&self, username: &str, reason: impl Into<String>) {
        let _ = self.broadcast_tx.send(BroadcastEvent::Kick {
            target_username_lower: username.to_ascii_lowercase(),
            reason: reason.into(),
        });
    }

    pub fn clock_snapshot(&self) -> (i64, i64) {
        (
            self.game_time.load(Ordering::Relaxed),
            self.day_time.load(Ordering::Relaxed),
        )
    }

    pub fn advance_world_clock(&self) -> (i64, i64) {
        let gt = self.game_time.fetch_add(1, Ordering::Relaxed) + 1;
        let prev_day = self.day_time.load(Ordering::Relaxed);
        let next_day = (prev_day + 1).rem_euclid(24_000);
        self.day_time.store(next_day, Ordering::Relaxed);
        (gt, next_day)
    }

    pub fn set_day_time(&self, day_ticks: i64) -> (i64, i64) {
        let normalized = day_ticks.rem_euclid(24_000);
        self.day_time.store(normalized, Ordering::Relaxed);
        (self.game_time.load(Ordering::Relaxed), normalized)
    }

    pub fn is_raining(&self) -> bool {
        self.raining.load(Ordering::Relaxed)
    }

    pub fn set_raining(&self, rain: bool) {
        self.raining.store(rain, Ordering::Relaxed);
    }

    /// Deposits or withdraws items from a `minecraft:chest` at `pos`, returning `(summary, inventory_changed)`.
    pub fn interact_chest(
        &self,
        entity_id: i32,
        pos: (i32, i32, i32),
        slot: usize,
    ) -> Option<(String, PlayerInventory)> {
        let mut pguard = self.players.write().expect("players lock");
        let player = pguard.get_mut(&entity_id)?;
        let mut cguard = self.chests.write().expect("chests lock");
        let chest = cguard.entry(pos).or_default();

        let held = player.inventory.main[slot];
        let msg = if !held.is_empty() {
            let leftover = chest.deposit(held);
            player.inventory.main[slot] = leftover;
            format!("[Chest] Stored stack -> contents: {}", chest.summary())
        } else if let Some(withdrawn) = chest.withdraw_first() {
            let leftover = player.inventory.insert(withdrawn);
            if !leftover.is_empty() {
                let _ = chest.deposit(leftover);
            }
            format!("[Chest] Withdrew item -> remaining: {}", chest.summary())
        } else {
            "[Chest] Chest is empty (hold an item & right-click to deposit).".to_owned()
        };
        Some((msg, player.inventory.clone()))
    }

    /// Interacts with a `minecraft:furnace` at `pos` (collects smelted output or loads ore/food/fuel).
    pub fn interact_furnace(
        &self,
        entity_id: i32,
        pos: (i32, i32, i32),
        slot: usize,
    ) -> Option<(String, PlayerInventory)> {
        let mut pguard = self.players.write().expect("players lock");
        let player = pguard.get_mut(&entity_id)?;
        let mut fguard = self.furnaces.write().expect("furnaces lock");
        let furnace = fguard.entry(pos).or_default();

        let held = player.inventory.main[slot];
        let (taken, collected) = furnace.interact(held);
        if taken > 0 {
            let _ = player.inventory.consume_slot(slot, taken);
        }
        if let Some(out_stack) = collected {
            let _ = player.inventory.insert(out_stack);
        }
        let summary = format!(
            "[Furnace] In: {}x | Fuel: {}x | Out: {}x | Burn: {}t",
            furnace.input.count, furnace.fuel.count, furnace.output.count, furnace.burn_ticks_left
        );
        Some((summary, player.inventory.clone()))
    }

    /// Removes any Chest or Furnace block entity at `pos` when broken, returning stored stacks to drop!
    pub fn drain_broken_block_entity(&self, pos: (i32, i32, i32)) -> Vec<ItemStack> {
        let mut out = Vec::new();
        if let Some(mut c) = self.chests.write().expect("chests lock").remove(&pos) {
            out.extend(c.drain_all());
        }
        if let Some(mut f) = self.furnaces.write().expect("furnaces lock").remove(&pos) {
            out.extend(f.drain_all());
        }
        out
    }

    pub fn loaded_chunk_count(&self) -> usize {
        self.chunks.read().expect("chunks lock").len()
    }

    pub fn with_chunks_read<R>(&self, f: impl FnOnce(&HashMap<ChunkPos, WorldChunk>) -> R) -> R {
        let guard = self.chunks.read().expect("chunks lock");
        f(&guard)
    }

    pub fn find_online_player(&self, username: &str) -> Option<ConnectedPlayer> {
        let target = username.to_ascii_lowercase();
        self.players
            .read()
            .expect("players lock")
            .values()
            .find(|p| p.username.to_ascii_lowercase() == target)
            .cloned()
    }

    pub async fn get_or_generate_chunk_packets_batch(
        &self,
        coords: &[ChunkPos],
    ) -> Result<Vec<(ChunkPos, Bytes)>> {
        let mut ready: Vec<(ChunkPos, Bytes)> = Vec::with_capacity(coords.len());
        let mut uncached: Vec<ChunkPos> = Vec::new();

        {
            let cache = self.chunk_packet_cache.read().expect("packet cache lock");
            for &pos in coords {
                if let Some(pkt) = cache.get(&pos) {
                    ready.push((pos, pkt.clone()));
                } else {
                    uncached.push(pos);
                }
            }
        }

        if uncached.is_empty() {
            return Ok(ready);
        }

        let mut modified_existing: Vec<WorldChunk> = Vec::new();
        let mut brand_new: Vec<ChunkPos> = Vec::new();
        {
            let guard = self.chunks.read().expect("chunks lock");
            for pos in uncached {
                if let Some(existing) = guard.get(&pos) {
                    modified_existing.push(existing.clone());
                } else {
                    brand_new.push(pos);
                }
            }
        }

        let generator = self.generator;
        let built = tokio::task::spawn_blocking(move || {
            let reencoded: Vec<(ChunkPos, Option<WorldChunk>, Bytes)> = modified_existing
                .into_par_iter()
                .map(|c| {
                    let pos = c.pos;
                    let pkt = Bytes::from(c.encode_level_chunk_with_light(false));
                    (pos, None, pkt)
                })
                .collect();

            let generated: Vec<(ChunkPos, Option<WorldChunk>, Bytes)> = generator
                .generate_and_encode_batch_parallel(&brand_new)
                .into_iter()
                .map(|(pos, chunk, pkt)| (pos, Some(chunk), Bytes::from(pkt)))
                .collect();

            let mut combined = reencoded;
            combined.extend(generated);
            combined
        })
        .await
        .context("join multi-core rayon chunk generation batch")?;

        {
            let mut chunks_guard = self.chunks.write().expect("chunks lock");
            let mut cache_guard = self.chunk_packet_cache.write().expect("packet cache lock");
            for (pos, maybe_new_chunk, pkt) in built {
                if let Some(new_chunk) = maybe_new_chunk {
                    chunks_guard.entry(pos).or_insert(new_chunk);
                }
                cache_guard.insert(pos, pkt.clone());
                ready.push((pos, pkt));
            }
        }

        Ok(ready)
    }

    pub fn get_block(&self, x: i32, y: i32, z: i32) -> u32 {
        let cpos = ChunkPos {
            x: x.div_euclid(16),
            z: z.div_euclid(16),
        };
        let lx = x.rem_euclid(16) as usize;
        let lz = z.rem_euclid(16) as usize;
        let guard = self.chunks.read().expect("chunks lock");
        guard
            .get(&cpos)
            .map(|c| c.get_world_block(lx, y, lz))
            .unwrap_or(generated_blocks::default_state::AIR)
    }

    pub fn set_block(&self, x: i32, y: i32, z: i32, state_id: u32) -> bool {
        if !(-64..320).contains(&y) {
            return false;
        }
        let cpos = ChunkPos {
            x: x.div_euclid(16),
            z: z.div_euclid(16),
        };
        let lx = x.rem_euclid(16) as usize;
        let lz = z.rem_euclid(16) as usize;

        {
            let mut guard = self.chunks.write().expect("chunks lock");
            let chunk = guard
                .entry(cpos)
                .or_insert_with(|| self.generator.generate_chunk(cpos));
            chunk.set_world_block(lx, y, lz, state_id);
        }
        self.chunk_packet_cache
            .write()
            .expect("packet cache lock")
            .remove(&cpos);
        self.dirty_chunks.write().expect("dirty lock").insert(cpos);
        true
    }

    pub fn snapshot_players(&self) -> Vec<ConnectedPlayer> {
        self.players
            .read()
            .expect("players lock")
            .values()
            .cloned()
            .collect()
    }

    pub fn snapshot_mobs(&self) -> Vec<WorldMob> {
        self.mobs
            .read()
            .expect("mobs lock")
            .values()
            .cloned()
            .collect()
    }

    pub fn snapshot_item_drops(&self) -> Vec<WorldItemDrop> {
        self.item_drops
            .read()
            .expect("item_drops lock")
            .values()
            .cloned()
            .collect()
    }

    pub fn snapshot_arrows(&self) -> Vec<WorldArrow> {
        self.arrows
            .read()
            .expect("arrows lock")
            .values()
            .cloned()
            .collect()
    }

    pub fn spawn_arrow(
        &self,
        entity_id: i32,
        shooter_entity_id: i32,
        eye_pos: [f64; 3],
        yaw_deg: f32,
        pitch_deg: f32,
    ) -> WorldArrow {
        let arrow = WorldArrow::from_aim(entity_id, shooter_entity_id, eye_pos, yaw_deg, pitch_deg);
        self.arrows
            .write()
            .expect("arrows lock")
            .insert(entity_id, arrow.clone());
        arrow
    }

    pub fn grant_player_xp(&self, entity_id: i32, points: i32) -> Option<(PlayerVitals, bool)> {
        let mut pguard = self.players.write().expect("players lock");
        let p = pguard.get_mut(&entity_id)?;
        let leveled = p.vitals.add_experience(points);
        Some((p.vitals, leveled))
    }

    pub fn spawn_mob(&self, entity_id: i32, kind: MobKind, pos: [f64; 3]) -> WorldMob {
        let mob = WorldMob::new(entity_id, kind, pos);
        self.mobs
            .write()
            .expect("mobs lock")
            .insert(entity_id, mob.clone());
        mob
    }

    pub fn spawn_item_drop(
        &self,
        entity_id: i32,
        item_id: u32,
        count: u8,
        pos: [f64; 3],
    ) -> WorldItemDrop {
        let now_tick = self.game_time.load(Ordering::Relaxed).max(0) as u64;
        let drop = WorldItemDrop::new(entity_id, item_id, count, pos, now_tick);
        self.item_drops
            .write()
            .expect("item_drops lock")
            .insert(entity_id, drop.clone());
        drop
    }

    pub fn add_player(&self, player: ConnectedPlayer) {
        self.players
            .write()
            .expect("players lock")
            .insert(player.entity_id, player);
    }

    pub fn update_player_transform(&self, entity_id: i32, pos: [f64; 3], yaw: f32, pitch: f32) {
        if let Some(p) = self
            .players
            .write()
            .expect("players lock")
            .get_mut(&entity_id)
        {
            p.position = pos;
            p.yaw_deg = yaw;
            p.pitch_deg = pitch;
        }
    }

    /// Updates player transform + fall distance; returns `Some((vitals, fall_dmg))` if fall damage occurred!
    pub fn record_player_movement(
        &self,
        entity_id: i32,
        pos: [f64; 3],
        yaw: f32,
        pitch: f32,
        on_ground: bool,
    ) -> Option<(PlayerVitals, f32)> {
        let mut guard = self.players.write().expect("players lock");
        let p = guard.get_mut(&entity_id)?;
        p.position = pos;
        p.yaw_deg = yaw;
        p.pitch_deg = pitch;
        let on_ladder = generated_blocks::block_by_state_id(self.get_block(
            pos[0].floor() as i32,
            pos[1].floor() as i32,
            pos[2].floor() as i32,
        ))
        .is_some_and(|b| b.name == "minecraft:ladder");
        if on_ladder {
            p.vitals.fall_distance = 0.0;
            p.vitals.last_y = pos[1];
            return None;
        }
        let vulnerable = matches!(p.game_mode, GameMode::Survival | GameMode::Adventure);
        let armor_pts = p.inventory.total_armor_points();
        let dmg = if armor_pts > 0 && on_ground && p.vitals.fall_distance > 3.25 && vulnerable {
            let dist = p.vitals.fall_distance;
            p.vitals.fall_distance = 0.0;
            p.vitals.last_y = pos[1];
            p.vitals
                .apply_damage_with_armor((dist - 3.0).ceil(), armor_pts / 2)
        } else {
            p.vitals.update_movement(pos[1], on_ground, vulnerable)
        };
        (dmg > 0.0).then_some((p.vitals, dmg))
    }

    pub fn mutate_player<R>(
        &self,
        entity_id: i32,
        f: impl FnOnce(&mut ConnectedPlayer) -> R,
    ) -> Option<R> {
        let mut guard = self.players.write().expect("players lock");
        guard.get_mut(&entity_id).map(f)
    }

    /// Applies melee damage & knockback against a `WorldMob`. Returns `(mob, hurt_yaw_deg, died)`.
    pub fn attack_mob(
        &self,
        mob_id: i32,
        attacker_pos: [f64; 3],
        damage: f32,
    ) -> Option<(WorldMob, f32, bool)> {
        let mut guard = self.mobs.write().expect("mobs lock");
        let mob = guard.get_mut(&mob_id)?;
        let (kb, hurt_yaw) = compute_melee_knockback(attacker_pos, mob.position, 0.55);
        mob.health = (mob.health - damage).max(0.0);
        let nx = mob.position[0] + kb[0];
        let nz = mob.position[2] + kb[2];
        let sy = (self
            .generator
            .surface_height(nx.floor() as i32, nz.floor() as i32)
            + 1) as f64;
        mob.position = [nx, sy, nz];
        let snapshot = mob.clone();
        let died = snapshot.health <= 0.0;
        if died {
            guard.remove(&mob_id);
        }
        Some((snapshot, hurt_yaw, died))
    }

    /// Applies melee PvP damage & knockback against an online player.
    /// Returns `(updated_victim, damage_dealt, hurt_yaw_deg)`.
    pub fn attack_player(
        &self,
        victim_id: i32,
        attacker_pos: [f64; 3],
        damage: f32,
    ) -> Option<(ConnectedPlayer, f32, f32)> {
        let mut guard = self.players.write().expect("players lock");
        let victim = guard.get_mut(&victim_id)?;
        if matches!(victim.game_mode, GameMode::Creative | GameMode::Spectator) {
            return None;
        }
        let armor_pts = victim.inventory.total_armor_points();
        let dealt = victim.vitals.apply_damage_with_armor(damage, armor_pts);
        if dealt <= 0.0 {
            return None;
        }
        let (kb, hurt_yaw) = compute_melee_knockback(attacker_pos, victim.position, 0.50);
        victim.position[0] += kb[0];
        victim.position[1] += kb[1];
        victim.position[2] += kb[2];
        Some((victim.clone(), dealt, hurt_yaw))
    }

    /// Steps mob wandering/pursuit AI, hostile zombie melee hits, floating item auto-pickup,
    /// and player natural health regeneration for one tick (`50 ms`).
    pub fn step_mobs_and_vitals(&self, tick_number: u64) -> WorldTickOutcome {
        let mut out = WorldTickOutcome::default();

        let survival_targets: Vec<(i32, [f64; 3])> = {
            let pguard = self.players.read().expect("players lock");
            pguard
                .values()
                .filter(|p| p.game_mode == GameMode::Survival && !p.vitals.is_dead())
                .map(|p| (p.entity_id, p.position))
                .collect()
        };

        // Tick active furnaces (smelting raw iron, sand, cobblestone, meats)
        {
            let mut fguard = self.furnaces.write().expect("furnaces lock");
            for furnace in fguard.values_mut() {
                let _ = furnace.tick_smelt();
            }
        }

        let mut pending_hostile_strikes: Vec<(i32, [f64; 3], f32)> = Vec::new();
        {
            let mut mguard = self.mobs.write().expect("mobs lock");
            for mob in mguard.values_mut() {
                let nearest = survival_targets.iter().copied().min_by(|(_, a), (_, b)| {
                    let da = (a[0] - mob.position[0]).powi(2) + (a[2] - mob.position[2]).powi(2);
                    let db = (b[0] - mob.position[0]).powi(2) + (b[2] - mob.position[2]).powi(2);
                    da.total_cmp(&db)
                });
                if mob.step_ai(tick_number, nearest.map(|(_, pos)| pos), |wx, wz| {
                    self.generator.surface_height(wx, wz)
                }) {
                    out.moved_mobs.push(mob.clone());
                }
                if mob.kind.is_hostile()
                    && let Some((target_pid, ppos)) = nearest
                {
                    let d2 = (ppos[0] - mob.position[0]).powi(2)
                        + (ppos[1] - mob.position[1]).powi(2)
                        + (ppos[2] - mob.position[2]).powi(2);
                    if d2 <= 1.65 * 1.65 {
                        pending_hostile_strikes.push((
                            target_pid,
                            mob.position,
                            mob.kind.melee_damage(),
                        ));
                    }
                }
            }
        }

        // Apply hostile mob melee strikes & natural survival regeneration
        {
            let mut pguard = self.players.write().expect("players lock");
            for (pid, mpos, dmg) in pending_hostile_strikes {
                if let Some(victim) = pguard.get_mut(&pid) {
                    let armor_pts = victim.inventory.total_armor_points();
                    if victim.vitals.apply_damage_with_armor(dmg, armor_pts) > 0.0 {
                        let (kb, hurt_yaw) = compute_melee_knockback(mpos, victim.position, 0.35);
                        victim.position[0] += kb[0];
                        victim.position[2] += kb[2];
                        out.zombie_hits.push((victim.clone(), hurt_yaw));
                    }
                }
            }
            for p in pguard.values_mut() {
                if p.vitals.tick_natural(tick_number) {
                    out.healed_players.push((p.entity_id, p.vitals));
                }
            }
        }

        // Step active 3D arrows (ballistic flight, block impact, mob/PvP hits, and shooter XP!)
        {
            let mut aguard = self.arrows.write().expect("arrows lock");
            let arrow_ids: Vec<i32> = aguard.keys().copied().collect();
            for aid in arrow_ids {
                let Some(mut arrow) = aguard.get(&aid).cloned() else {
                    continue;
                };
                arrow.step_flight();
                if arrow.is_expired() {
                    aguard.remove(&aid);
                    out.despawned_items.push(aid);
                    continue;
                }

                // Solid terrain impact check
                let st = self.get_block(
                    arrow.position[0].floor() as i32,
                    arrow.position[1].floor() as i32,
                    arrow.position[2].floor() as i32,
                );
                let passable = st == generated_blocks::default_state::AIR
                    || st == generated_blocks::default_state::WATER
                    || ((7243..=7306).contains(&st) && (((st - 7243) / 2) & 1) == 0)
                    || generated_blocks::block_by_state_id(st)
                        .is_some_and(|b| matches!(b.name, "minecraft:ladder" | "minecraft:torch"));
                if !passable {
                    aguard.remove(&aid);
                    out.despawned_items.push(aid);
                    continue;
                }

                // Check Overworld mob hit
                let hit_mob_id = {
                    let mguard = self.mobs.read().expect("mobs lock");
                    mguard
                        .values()
                        .find(|m| arrow.intersects_target(m.position, 0.42, 1.85))
                        .map(|m| m.entity_id)
                };
                if let Some(mid) = hit_mob_id {
                    aguard.remove(&aid);
                    out.despawned_items.push(aid);
                    if let Some((mob_snap, hyaw, died)) =
                        self.attack_mob(mid, arrow.prev_position, WorldArrow::BASE_DAMAGE)
                    {
                        if died
                            && let Some((v, _)) = self.grant_player_xp(
                                arrow.shooter_entity_id,
                                if mob_snap.kind.is_hostile() { 12 } else { 6 },
                            )
                        {
                            out.xp_updates.push((arrow.shooter_entity_id, v));
                        }
                        out.arrow_hit_mobs.push((mob_snap, hyaw, died));
                    }
                    continue;
                }

                // Check PvP player hit
                let hit_pid = {
                    let pguard = self.players.read().expect("players lock");
                    pguard
                        .values()
                        .find(|p| {
                            p.entity_id != arrow.shooter_entity_id
                                && matches!(p.game_mode, GameMode::Survival | GameMode::Adventure)
                                && !p.vitals.is_dead()
                                && arrow.intersects_target(p.position, 0.35, 1.8)
                        })
                        .map(|p| p.entity_id)
                };
                if let Some(pid) = hit_pid {
                    aguard.remove(&aid);
                    out.despawned_items.push(aid);
                    if let Some((victim, _, hyaw)) =
                        self.attack_player(pid, arrow.prev_position, WorldArrow::BASE_DAMAGE)
                    {
                        out.zombie_hits.push((victim, hyaw));
                    }
                    continue;
                }

                out.moved_arrows.push(arrow.clone());
                aguard.insert(aid, arrow);
            }
        }

        // Auto-pickup nearby floating item drops within 2.0 blocks!
        {
            let mut pguard = self.players.write().expect("players lock");
            let mut dguard = self.item_drops.write().expect("item_drops lock");
            let drop_ids: Vec<i32> = dguard.keys().copied().collect();
            for drop_id in drop_ids {
                let Some(drop) = dguard.get(&drop_id).cloned() else {
                    continue;
                };
                if tick_number > drop.spawn_tick + 6_000 {
                    dguard.remove(&drop_id);
                    out.despawned_items.push(drop_id);
                    continue;
                }
                if tick_number < drop.spawn_tick + 4 {
                    continue;
                }
                let collector_id = pguard
                    .values()
                    .filter(|p| !p.vitals.is_dead() && p.game_mode != GameMode::Spectator)
                    .find(|p| {
                        let dx = p.position[0] - drop.position[0];
                        let dy = (p.position[1] + 0.5) - drop.position[1];
                        let dz = p.position[2] - drop.position[2];
                        dx * dx + dy * dy + dz * dz <= 2.15 * 2.15
                    })
                    .map(|p| p.entity_id);

                if let Some(pid) = collector_id
                    && let Some(collector) = pguard.get_mut(&pid)
                {
                    let stack = ItemStack::new(drop.item_id, drop.count);
                    let leftover = collector.inventory.insert(stack);
                    let picked = drop.count.saturating_sub(leftover.count);
                    if picked > 0 {
                        if leftover.is_empty() {
                            dguard.remove(&drop_id);
                        } else if let Some(d) = dguard.get_mut(&drop_id) {
                            d.count = leftover.count;
                        }
                        out.picked_up_items.push((
                            drop_id,
                            pid,
                            picked,
                            collector.inventory.clone(),
                        ));
                    }
                }
            }
        }

        out
    }

    pub fn update_player_gamemode(&self, entity_id: i32, mode: GameMode) {
        if let Some(p) = self
            .players
            .write()
            .expect("players lock")
            .get_mut(&entity_id)
        {
            p.game_mode = mode;
        }
    }

    pub fn remove_player(&self, entity_id: i32) {
        self.players
            .write()
            .expect("players lock")
            .remove(&entity_id);
    }

    pub fn online_usernames(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .players
            .read()
            .expect("players lock")
            .values()
            .map(|p| p.username.clone())
            .collect();
        names.sort();
        names
    }

    pub async fn save_dirty_chunks(&self, force_all: bool) -> Result<usize> {
        let to_save: Vec<WorldChunk> = {
            let mut dirty = self.dirty_chunks.write().expect("dirty lock");
            let chunks = self.chunks.read().expect("chunks lock");
            if force_all {
                dirty.clear();
                chunks.values().cloned().collect()
            } else {
                let coords: Vec<ChunkPos> = dirty.drain().collect();
                coords
                    .into_iter()
                    .filter_map(|p| chunks.get(&p).cloned())
                    .collect()
            }
        };

        if to_save.is_empty() {
            return Ok(0);
        }

        let region_dir = self.region_dir.clone();
        tokio::task::spawn_blocking(move || -> Result<usize> {
            let now_secs = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as u32;

            let mut by_region: HashMap<(i32, i32), Vec<WorldChunk>> = HashMap::new();
            for c in to_save {
                by_region.entry(region_coords(c.pos)).or_default().push(c);
            }

            let mut total = 0usize;
            for ((rx, rz), list) in by_region {
                let path = region_file_path(&region_dir, rx, rz);
                let mut reg = AnvilRegion::load_or_create(&path)?;
                for c in &list {
                    reg.put_chunk(c, now_secs)?;
                    total += 1;
                }
                reg.save_atomic(&path)?;
            }
            Ok(total)
        })
        .await
        .context("join Anvil save worker")?
    }
}
