//! Folia-inspired multi-core regionized world tick scheduler and rolling TPS / MSPT profiler.

use crate::config::PerformanceConfig;
use crate::net::SharedServerState;
use rayon::prelude::*;
use sky_api::RegionTickSummary;
use sky_protocol::config_play::encode_block_update;
use sky_protocol::generated_packets;
use sky_world::terrain::worker_threads;
use sky_world::{ChunkPos, OVERWORLD_MIN_Y, generated_blocks};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tracing::{info, warn};

/// Spatial region coordinate grouping `(1 << shift) x (1 << shift)` chunks into an independent
/// parallel simulation unit (Folia region model).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RegionCoord {
    pub rx: i32,
    pub rz: i32,
}

impl RegionCoord {
    #[inline]
    pub const fn from_chunk(cpos: ChunkPos, shift: u8) -> Self {
        Self {
            rx: cpos.x >> shift,
            rz: cpos.z >> shift,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TickReport {
    pub tick_number: u64,
    pub tps_5s: f64,
    pub tps_1m: f64,
    pub last_mspt: f64,
    pub avg_mspt_5s: f64,
    pub p95_mspt_5s: f64,
    pub active_regions: usize,
    pub loaded_chunks: usize,
    pub worker_threads: usize,
}

/// Rolling 60-second (`1,200`-tick) MSPT & TPS ring-buffer profiler.
pub struct TickProfiler {
    target_tps: u32,
    inner: RwLock<ProfilerInner>,
}

struct ProfilerInner {
    tick_number: u64,
    samples_ms: VecDeque<f64>,
    active_regions: usize,
    loaded_chunks: usize,
}

impl TickProfiler {
    pub fn new(target_tps: u32) -> Self {
        Self {
            target_tps: target_tps.max(1),
            inner: RwLock::new(ProfilerInner {
                tick_number: 0,
                samples_ms: VecDeque::with_capacity(1200),
                active_regions: 0,
                loaded_chunks: 0,
            }),
        }
    }

    pub fn record_tick(&self, elapsed_ms: f64, active_regions: usize, loaded_chunks: usize) -> u64 {
        let mut g = self.inner.write().expect("profiler lock");
        g.tick_number += 1;
        if g.samples_ms.len() >= 1200 {
            g.samples_ms.pop_front();
        }
        g.samples_ms.push_back(elapsed_ms);
        g.active_regions = active_regions;
        g.loaded_chunks = loaded_chunks;
        g.tick_number
    }

    pub fn snapshot(&self) -> TickReport {
        let g = self.inner.read().expect("profiler lock");
        let max_tps = f64::from(self.target_tps);
        let budget_ms = 1000.0 / max_tps;

        let last_mspt = g.samples_ms.back().copied().unwrap_or(0.0);

        let window_5s = (self.target_tps as usize * 5).min(g.samples_ms.len());
        let (avg_5s, p95_5s) = if window_5s == 0 {
            (0.0, 0.0)
        } else {
            let mut recent: Vec<f64> = g.samples_ms.iter().rev().take(window_5s).copied().collect();
            let sum: f64 = recent.iter().sum();
            recent.sort_by(f64::total_cmp);
            let p95_idx = ((recent.len() * 95) / 100).min(recent.len() - 1);
            (sum / (window_5s as f64), recent[p95_idx])
        };

        let avg_1m = if g.samples_ms.is_empty() {
            0.0
        } else {
            g.samples_ms.iter().sum::<f64>() / (g.samples_ms.len() as f64)
        };

        let tps_5s = if avg_5s <= budget_ms {
            max_tps
        } else {
            (1000.0 / avg_5s).min(max_tps)
        };
        let tps_1m = if avg_1m <= budget_ms {
            max_tps
        } else {
            (1000.0 / avg_1m).min(max_tps)
        };

        TickReport {
            tick_number: g.tick_number,
            tps_5s,
            tps_1m,
            last_mspt,
            avg_mspt_5s: avg_5s,
            p95_mspt_5s: p95_5s,
            active_regions: g.active_regions,
            loaded_chunks: g.loaded_chunks,
            worker_threads: worker_threads(),
        }
    }
}

#[inline]
fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// Partitions active simulation chunks into disjoint Folia `RegionCoord` buckets and ticks every
/// region concurrently across `rayon` CPU worker threads.
pub fn run_parallel_region_tick(
    state: &SharedServerState,
    tick_number: u64,
    perf: &PerformanceConfig,
) -> (usize, usize) {
    let shift = perf.region_shift_chunks.min(6);
    let sim_radius = i32::from(state.props.simulation_distance.clamp(2, 12));
    let players = state.world.snapshot_players();

    let mut active_chunks = HashSet::new();
    if players.is_empty() {
        // Keep spawn chunk (0, 0) region warm even while idle
        active_chunks.insert(ChunkPos { x: 0, z: 0 });
    } else {
        for p in &players {
            let pcx = (p.position[0].floor() as i32).div_euclid(16);
            let pcz = (p.position[2].floor() as i32).div_euclid(16);
            for dz in -sim_radius..=sim_radius {
                for dx in -sim_radius..=sim_radius {
                    active_chunks.insert(ChunkPos {
                        x: pcx + dx,
                        z: pcz + dz,
                    });
                }
            }
        }
    }

    // Group active chunks into disjoint Folia spatial regions
    let mut regions: HashMap<RegionCoord, Vec<ChunkPos>> = HashMap::new();
    for cpos in active_chunks {
        regions
            .entry(RegionCoord::from_chunk(cpos, shift))
            .or_default()
            .push(cpos);
    }
    let region_count = regions.len();
    let region_vec: Vec<(RegionCoord, Vec<ChunkPos>)> = regions.into_iter().collect();

    let rts = perf.random_tick_speed;
    if rts == 0 {
        return (region_count, 0);
    }

    let dirt = generated_blocks::default_state::DIRT;
    let grass = generated_blocks::default_state::GRASS_BLOCK;
    let air = generated_blocks::default_state::AIR;

    // Parallel Folia region execution across all CPU cores!
    let updates: Vec<(i32, i32, i32, u32)> = state.world.with_chunks_read(|chunk_map| {
        region_vec
            .par_iter()
            .flat_map_iter(|(rcoord, chunk_coords)| {
                let mut local_updates = Vec::new();
                let mut rng = splitmix64(
                    tick_number ^ ((rcoord.rx as i64 as u64) << 32) ^ (rcoord.rz as i64 as u64),
                );

                for &cpos in chunk_coords {
                    let Some(chunk) = chunk_map.get(&cpos) else {
                        continue;
                    };
                    for (sec_idx, sec) in chunk.sections.iter().enumerate() {
                        if sec.is_empty() {
                            continue;
                        }
                        let base_y = OVERWORLD_MIN_Y + (sec_idx as i32) * 16;
                        for _ in 0..rts {
                            rng = splitmix64(rng);
                            let lx = (rng & 0xF) as usize;
                            let ly = ((rng >> 4) & 0xF) as usize;
                            let lz = ((rng >> 8) & 0xF) as usize;
                            if sec.get(lx, ly, lz) != dirt {
                                continue;
                            }
                            let wy = base_y + ly as i32;
                            if chunk.get_world_block(lx, wy + 1, lz) != air {
                                continue;
                            }
                            // Check if an in-chunk neighbor has grass to spread from
                            let has_grass_neighbor = (lx > 0
                                && chunk.get_world_block(lx - 1, wy, lz) == grass)
                                || (lx < 15 && chunk.get_world_block(lx + 1, wy, lz) == grass)
                                || (lz > 0 && chunk.get_world_block(lx, wy, lz - 1) == grass)
                                || (lz < 15 && chunk.get_world_block(lx, wy, lz + 1) == grass);
                            if has_grass_neighbor {
                                let wx = cpos.x * 16 + lx as i32;
                                let wz = cpos.z * 16 + lz as i32;
                                local_updates.push((wx, wy, wz, grass));
                            }
                        }
                    }
                }
                local_updates
            })
            .collect()
    });

    let applied = updates.len();
    for (wx, wy, wz, new_st) in updates {
        if state.world.set_block(wx, wy, wz, new_st) {
            state.world.broadcast_all(
                generated_packets::play::clientbound::BLOCK_UPDATE,
                encode_block_update(wx, wy, wz, new_st),
            );
        }
    }

    (region_count, applied)
}

/// Spawns the dedicated `20 TPS` Folia-style regionized tick loop off the async network tasks.
pub fn spawn_region_tick_loop(state: Arc<SharedServerState>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let tps = state.sky_cfg.performance.target_tps.clamp(1, 100);
        let period = Duration::from_nanos(1_000_000_000 / u64::from(tps));
        let mut interval = tokio::time::interval(period);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        let mut next_tick = 1u64;
        loop {
            interval.tick().await;
            let st_clone = Arc::clone(&state);
            let tick_id = next_tick;
            next_tick += 1;

            let tick_res = tokio::task::spawn_blocking(move || {
                let t0 = Instant::now();
                let (regions, updates) =
                    run_parallel_region_tick(&st_clone, tick_id, &st_clone.sky_cfg.performance);
                let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;
                let loaded = st_clone.world.loaded_chunk_count();
                st_clone.profiler.record_tick(elapsed_ms, regions, loaded);
                let report = st_clone.profiler.snapshot();
                st_clone.plugins.fire_tick(&RegionTickSummary {
                    tick_number: report.tick_number,
                    active_regions: report.active_regions,
                    random_updates_applied: updates,
                    last_mspt: report.last_mspt,
                    tps_5s: report.tps_5s,
                });
            })
            .await;

            if let Err(err) = tick_res {
                warn!(error = %err, "parallel region tick worker panicked");
            }

            let autosave = state.sky_cfg.performance.autosave_interval_ticks;
            if autosave > 0 && tick_id.is_multiple_of(autosave) {
                match state.world.save_dirty_chunks(false).await {
                    Ok(saved) if saved > 0 => {
                        info!(
                            saved_chunks = saved,
                            tick = tick_id,
                            "periodic Anvil autosave complete"
                        );
                    }
                    Ok(_) => {}
                    Err(err) => warn!(error = %err, "periodic Anvil autosave failed"),
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiler_computes_20_tps_under_budget_and_scales_over_budget() {
        let prof = TickProfiler::new(20);
        for _ in 0..20 {
            prof.record_tick(1.25, 4, 81);
        }
        let fast = prof.snapshot();
        assert_eq!(fast.tick_number, 20);
        assert!((fast.tps_5s - 20.0).abs() < 1e-6);
        assert!(fast.avg_mspt_5s < 2.0);
        assert_eq!(fast.active_regions, 4);

        let overloaded = TickProfiler::new(20);
        for _ in 0..20 {
            overloaded.record_tick(100.0, 8, 400); // 100ms/tick -> 10.0 TPS
        }
        let slow = overloaded.snapshot();
        assert!((slow.tps_5s - 10.0).abs() < 1e-4);
    }
}
