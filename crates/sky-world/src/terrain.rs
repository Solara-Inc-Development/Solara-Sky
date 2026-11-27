//! Multi-octave procedural Overworld terrain, cave, beach, lake, and oak tree generator
//! with `rayon` multi-core parallel chunk + lighting batch generation.

use crate::{ChunkPos, WorldChunk, generated_blocks};
use rayon::prelude::*;

pub const SEA_LEVEL: i32 = -56;

/// Returns the number of active `rayon` CPU worker threads used for parallel worldgen & lighting.
pub fn worker_threads() -> usize {
    rayon::current_num_threads()
}

#[inline]
fn hash3(seed: u64, x: i32, y: i32, z: i32) -> f64 {
    let mut h = seed
        ^ (x as i64 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (y as i64 as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9)
        ^ (z as i64 as u64).wrapping_mul(0x94D0_49BB_1331_11EB);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h = (h ^ (h >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^= h >> 31;
    // Map to [-1.0, 1.0]
    ((h >> 11) as f64) * (2.0 / ((1u64 << 53) as f64)) - 1.0
}

#[inline]
fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

#[inline]
fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + t * (b - a)
}

fn value_noise_2d(seed: u64, x: f64, z: f64) -> f64 {
    let xi = x.floor() as i32;
    let zi = z.floor() as i32;
    let xf = fade(x - xi as f64);
    let zf = fade(z - zi as f64);

    let v00 = hash3(seed, xi, 0, zi);
    let v10 = hash3(seed, xi + 1, 0, zi);
    let v01 = hash3(seed, xi, 0, zi + 1);
    let v11 = hash3(seed, xi + 1, 0, zi + 1);

    lerp(lerp(v00, v10, xf), lerp(v01, v11, xf), zf)
}

fn value_noise_3d(seed: u64, x: f64, y: f64, z: f64) -> f64 {
    let xi = x.floor() as i32;
    let yi = y.floor() as i32;
    let zi = z.floor() as i32;
    let xf = fade(x - xi as f64);
    let yf = fade(y - yi as f64);
    let zf = fade(z - zi as f64);

    let c000 = hash3(seed, xi, yi, zi);
    let c100 = hash3(seed, xi + 1, yi, zi);
    let c010 = hash3(seed, xi, yi + 1, zi);
    let c110 = hash3(seed, xi + 1, yi + 1, zi);
    let c001 = hash3(seed, xi, yi, zi + 1);
    let c101 = hash3(seed, xi + 1, yi, zi + 1);
    let c011 = hash3(seed, xi, yi + 1, zi + 1);
    let c111 = hash3(seed, xi + 1, yi + 1, zi + 1);

    let x00 = lerp(c000, c100, xf);
    let x10 = lerp(c010, c110, xf);
    let x01 = lerp(c001, c101, xf);
    let x11 = lerp(c011, c111, xf);

    lerp(lerp(x00, x10, yf), lerp(x01, x11, yf), zf)
}

fn fbm_2d(seed: u64, mut x: f64, mut z: f64, octaves: usize) -> f64 {
    let mut sum = 0.0;
    let mut amp = 0.5;
    let mut norm = 0.0;
    for oct in 0..octaves {
        sum += amp * value_noise_2d(seed.wrapping_add(oct as u64 * 1013), x, z);
        norm += amp;
        x *= 2.03;
        z *= 2.03;
        amp *= 0.5;
    }
    sum / norm
}

/// Deterministic procedural Overworld terrain generator.
#[derive(Debug, Clone, Copy)]
pub struct TerrainGenerator {
    pub seed: u64,
}

impl TerrainGenerator {
    pub const fn new(seed: u64) -> Self {
        Self { seed }
    }

    /// Computes surface height at world `(wx, wz)`, smoothly flattening near the `(8, 8)` spawn plaza.
    pub fn surface_height(&self, wx: i32, wz: i32) -> i32 {
        let dx = (wx - 8) as f64;
        let dz = (wz - 8) as f64;
        let dist = (dx * dx + dz * dz).sqrt();

        // Keep spawn clearing flat at y = -60 within radius 12, smoothly blending out to radius 28
        let blend = ((dist - 12.0) / 16.0).clamp(0.0, 1.0);
        if blend <= 0.0 {
            return -60;
        }

        let continental = fbm_2d(self.seed, wx as f64 * 0.008, wz as f64 * 0.008, 4);
        let detail = fbm_2d(
            self.seed ^ 0xA5A5_5A5A,
            wx as f64 * 0.032,
            wz as f64 * 0.032,
            3,
        );
        let raw_y = -52.0 + continental * 16.0 + detail * 6.0;
        let blended = lerp(-60.0, raw_y, blend);
        (blended.round() as i32).clamp(-62, -24)
    }

    pub fn generate_chunk(&self, pos: ChunkPos) -> WorldChunk {
        let mut chunk = WorldChunk::new_empty(pos);

        let bedrock = generated_blocks::default_state::BEDROCK;
        let deepslate = generated_blocks::default_state::DEEPSLATE;
        let stone = generated_blocks::default_state::STONE;
        let dirt = generated_blocks::default_state::DIRT;
        let grass = generated_blocks::default_state::GRASS_BLOCK;
        let sand = generated_blocks::default_state::SAND;
        let water = generated_blocks::default_state::WATER;
        let cobble = generated_blocks::default_state::COBBLESTONE;
        let planks = generated_blocks::default_state::OAK_PLANKS;
        let log = generated_blocks::default_state::OAK_LOG;
        let leaves = generated_blocks::default_state::OAK_LEAVES;
        let glowstone = generated_blocks::default_state::GLOWSTONE;
        let coal_ore =
            generated_blocks::default_state_by_name("minecraft:coal_ore").unwrap_or(stone);
        let iron_ore =
            generated_blocks::default_state_by_name("minecraft:iron_ore").unwrap_or(stone);
        let gold_ore = generated_blocks::default_state_by_name("minecraft:deepslate_gold_ore")
            .unwrap_or(deepslate);
        let diamond_ore =
            generated_blocks::default_state_by_name("minecraft:deepslate_diamond_ore")
                .unwrap_or(deepslate);

        let mut heights = [0i32; 256];

        for lz in 0..16 {
            let wz = pos.z * 16 + lz as i32;
            for lx in 0..16 {
                let wx = pos.x * 16 + lx as i32;
                let top_y = self.surface_height(wx, wz);
                heights[lz * 16 + lx] = top_y;

                chunk.set_world_block(lx, -64, lz, bedrock);

                let is_beach = top_y <= SEA_LEVEL + 1;
                for y in -63..=top_y {
                    // Carve 3D caves safely below surface_y - 4
                    if y <= top_y - 5 && y > -63 {
                        let cave_n = value_noise_3d(
                            self.seed ^ 0xCAFE_BABE,
                            wx as f64 * 0.07,
                            y as f64 * 0.09,
                            wz as f64 * 0.07,
                        );
                        if cave_n > 0.56 {
                            // Occasional glowing crystal on cave roofs
                            if y == top_y - 5 && hash3(self.seed ^ 0x777, wx, y, wz) > 0.92 {
                                chunk.set_world_block(lx, y, lz, glowstone);
                            }
                            continue;
                        }
                    }

                    let block = if y == top_y {
                        if is_beach { sand } else { grass }
                    } else if y >= top_y - 3 {
                        if is_beach { sand } else { dirt }
                    } else if y < -52 {
                        let ore_roll = hash3(self.seed ^ 0x0E2E_D1A0, wx, y, wz);
                        if y <= -57 && ore_roll > 0.985 {
                            diamond_ore
                        } else if ore_roll > 0.965 {
                            gold_ore
                        } else {
                            deepslate
                        }
                    } else {
                        let ore_roll = hash3(self.seed ^ 0x0E2E_C0A1, wx, y, wz);
                        if ore_roll > 0.968 {
                            iron_ore
                        } else if ore_roll > 0.935 {
                            coal_ore
                        } else {
                            stone
                        }
                    };
                    chunk.set_world_block(lx, y, lz, block);
                }

                if top_y < SEA_LEVEL && !(pos.x == 0 && pos.z == 0) {
                    // Only fill lakes outside the lowered spawn sanctuary
                    let dx = (wx - 8) as f64;
                    let dz = (wz - 8) as f64;
                    if dx * dx + dz * dz > 20.0 * 20.0 {
                        for wy in (top_y + 1)..=SEA_LEVEL {
                            chunk.set_world_block(lx, wy, lz, water);
                        }
                    }
                }
            }
        }

        // Preserve the iconic spawn plaza in chunk (0, 0)
        if pos.x == 0 && pos.z == 0 {
            for lz in 4..=11 {
                for lx in 4..=11 {
                    let border = lx == 4 || lx == 11 || lz == 4 || lz == 11;
                    chunk.set_world_block(lx, -59, lz, if border { cobble } else { planks });
                }
            }
            for &(px, pz) in &[(4, 4), (4, 11), (11, 4), (11, 11)] {
                for y in -58..=-55 {
                    chunk.set_world_block(px, y, pz, log);
                }
                chunk.set_world_block(px, -54, pz, glowstone);
            }
        } else {
            // Place procedural oak trees safely inside chunk interior (2..14) so crowns don't clip borders
            for lz in 3..13 {
                for lx in 3..13 {
                    let wx = pos.x * 16 + lx as i32;
                    let wz = pos.z * 16 + lz as i32;
                    let top_y = heights[lz * 16 + lx];
                    if top_y <= SEA_LEVEL + 2 {
                        continue;
                    }
                    if hash3(self.seed ^ 0x1234_5678, wx, top_y, wz) > 0.965 {
                        let trunk_h = 4 + ((wx.abs() + wz.abs()) % 2);
                        for ty in 1..=trunk_h {
                            chunk.set_world_block(lx, top_y + ty, lz, log);
                        }
                        let crown_base = top_y + trunk_h - 1;
                        for cy in crown_base..=(crown_base + 2) {
                            let rad: i32 = if cy == crown_base + 2 { 1 } else { 2 };
                            for dz in -rad..=rad {
                                for dx in -rad..=rad {
                                    if dx == 0 && dz == 0 && cy <= top_y + trunk_h {
                                        continue;
                                    }
                                    if dx.abs() == rad && dz.abs() == rad && cy > crown_base {
                                        continue;
                                    }
                                    let tx = (lx as i32 + dx) as usize;
                                    let tz = (lz as i32 + dz) as usize;
                                    chunk.set_world_block(tx, cy, tz, leaves);
                                }
                            }
                        }
                    }
                }
            }
        }

        chunk
    }

    /// Generates procedural terrain, computes 24-section BFS Sky & Block lighting, and serializes
    /// MC `26.3` `LevelChunkWithLight` packets concurrently across all CPU cores via `rayon`.
    pub fn generate_and_encode_batch_parallel(
        &self,
        coords: &[ChunkPos],
    ) -> Vec<(ChunkPos, WorldChunk, Vec<u8>)> {
        coords
            .par_iter()
            .map(|&pos| {
                let chunk = self.generate_chunk(pos);
                let encoded = chunk.encode_level_chunk_with_light(false);
                (pos, chunk, encoded)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multi_core_batch_generation_matches_deterministic_seed() {
        let terrain_gen = TerrainGenerator::new(0x501A_2A26);
        let coords = [
            ChunkPos { x: -1, z: -1 },
            ChunkPos { x: 0, z: 0 },
            ChunkPos { x: 1, z: 2 },
            ChunkPos { x: 3, z: -2 },
        ];
        let batch = terrain_gen.generate_and_encode_batch_parallel(&coords);
        assert_eq!(batch.len(), coords.len());
        for (pos, _chunk, encoded) in batch {
            let expected = terrain_gen
                .generate_chunk(pos)
                .encode_level_chunk_with_light(false);
            assert_eq!(encoded, expected);
        }
    }

    #[test]
    fn procedural_terrain_generates_varied_hills_and_keeps_spawn_plaza() {
        let terrain_gen = TerrainGenerator::new(0x501A_2A26);
        assert_eq!(terrain_gen.surface_height(8, 8), -60);

        let mut min_h = i32::MAX;
        let mut max_h = i32::MIN;
        for z in -120..120 {
            for x in -120..120 {
                let h = terrain_gen.surface_height(x, z);
                min_h = min_h.min(h);
                max_h = max_h.max(h);
            }
        }
        assert!(
            max_h - min_h >= 6,
            "expected rolling elevation range, got {min_h}..={max_h}"
        );

        let spawn_chunk = terrain_gen.generate_chunk(ChunkPos { x: 0, z: 0 });
        assert_eq!(
            spawn_chunk.get_world_block(7, -59, 7),
            generated_blocks::default_state::OAK_PLANKS
        );
    }
}
