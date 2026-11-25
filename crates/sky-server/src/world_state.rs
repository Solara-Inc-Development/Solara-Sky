use anyhow::{Context, Result};
use bytes::Bytes;
use rayon::prelude::*;
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
}

/// Thread-safe multi-core procedural Overworld chunk store, zero-copy `Bytes` packet cache,
/// multiplayer entity roster, and asynchronous Anvil (`.mca`) persistence.
pub struct LiveWorld {
    region_dir: PathBuf,
    generator: TerrainGenerator,
    chunks: RwLock<HashMap<ChunkPos, WorldChunk>>,
    chunk_packet_cache: RwLock<HashMap<ChunkPos, Bytes>>,
    dirty_chunks: RwLock<HashSet<ChunkPos>>,
    players: RwLock<HashMap<i32, ConnectedPlayer>>,
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

        // Multi-core lighting + network packet encoding for restored Anvil chunks
        let encoded_disk: Vec<(ChunkPos, WorldChunk, Vec<u8>)> = disk_chunks
            .into_par_iter()
            .map(|chunk| {
                let pos = chunk.pos;
                let pkt = chunk.encode_level_chunk_with_light(false);
                (pos, chunk, pkt)
            })
            .collect();

        // Multi-core procedural terrain + 24-section BFS lighting + packet encoding for new chunks
        let encoded_generated = generator.generate_and_encode_batch_parallel(&missing_coords);

        let total_cap = encoded_disk.len() + encoded_generated.len();
        let mut chunks = HashMap::with_capacity(total_cap);
        let mut packet_cache = HashMap::with_capacity(total_cap);

        for (pos, chunk, pkt) in encoded_disk.into_iter().chain(encoded_generated) {
            packet_cache.insert(pos, Bytes::from(pkt));
            chunks.insert(pos, chunk);
        }

        info!(
            worker_threads = worker_threads(),
            generated_chunks = generated_count,
            loaded_from_disk,
            "multi-core spawn world initialization complete"
        );

        let (broadcast_tx, _) = broadcast::channel(1024);
        Ok(Self {
            region_dir,
            generator,
            chunks: RwLock::new(chunks),
            chunk_packet_cache: RwLock::new(packet_cache),
            dirty_chunks: RwLock::new(HashSet::new()),
            players: RwLock::new(HashMap::new()),
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

    /// Fetches zero-copy `Bytes` chunk packets for `coords`. Cache misses (newly explored chunks or
    /// invalidated modified chunks) are generated, lit, and serialized in parallel across all CPU cores
    /// on the `rayon` pool via `spawn_blocking` without ever stalling async network tasks.
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

        // Partition uncached coords into already-existing (modified) chunks vs brand-new chunks
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
