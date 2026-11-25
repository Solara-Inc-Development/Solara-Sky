use anyhow::{Context, Result};
use sky_protocol::config_play::GameMode;
use sky_world::anvil::{AnvilRegion, region_coords, region_file_path};
use sky_world::{ChunkPos, WorldChunk, generated_blocks};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;
use tracing::info;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub enum BroadcastEvent {
    Packet { packet_id: i32, payload: Vec<u8> },
}

#[derive(Debug, Clone)]
pub struct ConnectedPlayer {
    pub entity_id: i32,
    pub username: String,
    pub uuid: Uuid,
    pub position: [f64; 3],
    pub game_mode: GameMode,
}

/// Thread-safe live Overworld chunk map, player roster, packet broadcast bus,
/// and asynchronous Anvil (`.mca`) region persistence.
pub struct LiveWorld {
    region_dir: PathBuf,
    chunks: RwLock<HashMap<ChunkPos, WorldChunk>>,
    dirty_chunks: RwLock<HashSet<ChunkPos>>,
    players: RwLock<HashMap<i32, ConnectedPlayer>>,
    broadcast_tx: broadcast::Sender<BroadcastEvent>,
}

impl LiveWorld {
    pub fn load_or_generate(region_dir: PathBuf, spawn_radius: i32) -> Result<Self> {
        std::fs::create_dir_all(&region_dir)
            .with_context(|| format!("create region dir {}", region_dir.display()))?;

        let mut region_cache: HashMap<(i32, i32), AnvilRegion> = HashMap::new();
        let mut chunks = HashMap::new();
        let mut loaded_from_disk = 0usize;

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
                    chunks.insert(pos, saved);
                    loaded_from_disk += 1;
                } else {
                    chunks.insert(pos, WorldChunk::generate_spawn_world(pos));
                }
            }
        }

        if loaded_from_disk > 0 {
            info!(
                loaded_from_disk,
                "restored chunks from existing Anvil .mca region files"
            );
        }

        let (broadcast_tx, _) = broadcast::channel(512);
        Ok(Self {
            region_dir,
            chunks: RwLock::new(chunks),
            dirty_chunks: RwLock::new(HashSet::new()),
            players: RwLock::new(HashMap::new()),
            broadcast_tx,
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<BroadcastEvent> {
        self.broadcast_tx.subscribe()
    }

    pub fn broadcast_packet(&self, packet_id: i32, payload: Vec<u8>) {
        let _ = self
            .broadcast_tx
            .send(BroadcastEvent::Packet { packet_id, payload });
    }

    pub fn serialize_spawn_chunks(&self) -> Vec<Vec<u8>> {
        let guard = self.chunks.read().expect("chunks lock");
        let mut coords: Vec<ChunkPos> = guard.keys().copied().collect();
        coords.sort_by_key(|p| (p.z, p.x));
        coords
            .into_iter()
            .filter_map(|p| {
                guard
                    .get(&p)
                    .map(|c| c.encode_level_chunk_with_light(false))
            })
            .collect()
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
                .or_insert_with(|| WorldChunk::generate_spawn_world(cpos));
            chunk.set_world_block(lx, y, lz, state_id);
        }
        {
            let mut dirty = self.dirty_chunks.write().expect("dirty lock");
            dirty.insert(cpos);
        }
        true
    }

    pub fn add_player(&self, player: ConnectedPlayer) {
        self.players
            .write()
            .expect("players lock")
            .insert(player.entity_id, player);
    }

    pub fn update_player_pos(&self, entity_id: i32, pos: [f64; 3]) {
        if let Some(p) = self
            .players
            .write()
            .expect("players lock")
            .get_mut(&entity_id)
        {
            p.position = pos;
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

    /// Saves all modified (or all loaded if `force_all`) chunks to Anvil `.mca` files on a blocking worker thread.
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
