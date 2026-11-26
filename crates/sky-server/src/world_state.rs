use anyhow::{Context, Result};
use bytes::Bytes;
use rayon::prelude::*;
use sky_entity::{MobKind, PlayerVitals, WorldMob, compute_melee_knockback};
use sky_inventory::PlayerInventory;
use sky_protocol::config_play::GameMode;
use sky_protocol::packets::ProfileProperty;
use sky_world::anvil::{AnvilRegion, region_coords, region_file_path};
use sky_world::terrain::{TerrainGenerator, worker_threads};
use sky_world::{ChunkPos, WorldChunk, generated_blocks};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::RwLock;
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

/// Thread-safe multi-core procedural Overworld chunk store, zero-copy `Bytes` packet cache,
/// survival player roster, world mobs, and asynchronous Anvil (`.mca`) persistence.
pub struct LiveWorld {
    region_dir: PathBuf,
    generator: TerrainGenerator,
    chunks: RwLock<HashMap<ChunkPos, WorldChunk>>,
    chunk_packet_cache: RwLock<HashMap<ChunkPos, Bytes>>,
    dirty_chunks: RwLock<HashSet<ChunkPos>>,
    players: RwLock<HashMap<i32, ConnectedPlayer>>,
    mobs: RwLock<HashMap<i32, WorldMob>>,
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

        // Seed starter Overworld mobs around the spawn meadows (entity IDs 1..=4)
        let mut initial_mobs = HashMap::new();
        let starter_spawns = [
            (1, MobKind::Pig, 16, 8),
            (2, MobKind::Cow, -4, 14),
            (3, MobKind::Sheep, 12, -6),
            (4, MobKind::Zombie, 22, 18),
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

    pub fn spawn_mob(&self, entity_id: i32, kind: MobKind, pos: [f64; 3]) -> WorldMob {
        let mob = WorldMob::new(entity_id, kind, pos);
        self.mobs
            .write()
            .expect("mobs lock")
            .insert(entity_id, mob.clone());
        mob
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
        let vulnerable = matches!(p.game_mode, GameMode::Survival | GameMode::Adventure);
        let dmg = p.vitals.update_movement(pos[1], on_ground, vulnerable);
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
        let dealt = victim.vitals.apply_damage(damage);
        if dealt <= 0.0 {
            return None;
        }
        let (kb, hurt_yaw) = compute_melee_knockback(attacker_pos, victim.position, 0.50);
        victim.position[0] += kb[0];
        victim.position[1] += kb[1];
        victim.position[2] += kb[2];
        Some((victim.clone(), dealt, hurt_yaw))
    }

    /// Steps mob wandering/pursuit AI and player natural vitals for one tick (`50 ms`).
    pub fn step_mobs_and_vitals(
        &self,
        tick_number: u64,
    ) -> (Vec<WorldMob>, Vec<(i32, PlayerVitals)>) {
        let survival_positions: Vec<[f64; 3]> = {
            let pguard = self.players.read().expect("players lock");
            pguard
                .values()
                .filter(|p| p.game_mode == GameMode::Survival && !p.vitals.is_dead())
                .map(|p| p.position)
                .collect()
        };

        let mut moved_mobs = Vec::new();
        {
            let mut mguard = self.mobs.write().expect("mobs lock");
            for mob in mguard.values_mut() {
                let nearest = survival_positions.iter().copied().min_by(|a, b| {
                    let da = (a[0] - mob.position[0]).powi(2) + (a[2] - mob.position[2]).powi(2);
                    let db = (b[0] - mob.position[0]).powi(2) + (b[2] - mob.position[2]).powi(2);
                    da.total_cmp(&db)
                });
                if mob.step_ai(tick_number, nearest, |wx, wz| {
                    self.generator.surface_height(wx, wz)
                }) {
                    moved_mobs.push(mob.clone());
                }
            }
        }

        let mut healed_players = Vec::new();
        {
            let mut pguard = self.players.write().expect("players lock");
            for p in pguard.values_mut() {
                if p.vitals.tick_natural(tick_number) {
                    healed_players.push((p.entity_id, p.vitals));
                }
            }
        }

        (moved_mobs, healed_players)
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
