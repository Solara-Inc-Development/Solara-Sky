//! Multi-octave procedural Overworld terrain, 5-biome climate system (Plains, Forest, Desert,
//! Snowy Taiga, Rocky Peaks), caves, ores, lakes, trees, and surface ruins with `rayon` multi-core
//! parallel chunk + lighting batch generation.

use crate::biomes::SurfaceBiome;
use crate::structures::{place_desert_well, place_watchtower_ruin};
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

/// Deterministic procedural Overworld terrain & multi-biome generator.
#[derive(Debug, Clone, Copy)]
pub struct TerrainGenerator {
    pub seed: u64,
}

impl TerrainGenerator {
    pub const fn new(seed: u64) -> Self {
        Self { seed }
    }

    pub fn biome_at(&self, wx: i32, wz: i32, top_y: i32) -> SurfaceBiome {
        let dx = (wx - 8) as f64;
        let dz = (wz - 8) as f64;
        let dist = (dx * dx + dz * dz).sqrt();
        let temp = fbm_2d(
            self.seed ^ 0x7E49_1111,
            wx as f64 * 0.0055,
            wz as f64 * 0.0055,
            3,
        );
        let humid = fbm_2d(
            self.seed ^ 0x40AD_2222,
            wx as f64 * 0.0065,
            wz as f64 * 0.0065,
            3,
        );
        SurfaceBiome::classify(temp, humid, top_y, dist)
    }

    /// Computes surface height at world `(wx, wz)`, smoothly flattening near the `(8, 8)` spawn plaza.
    pub fn surface_height(&self, wx: i32, wz: i32) -> i32 {
        let dx = (wx - 8) as f64;
        let dz = (wz - 8) as f64;
        let dist = (dx * dx + dz * dz).sqrt();

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
        let ridge = fbm_2d(
            self.seed ^ 0x9988_7766,
            wx as f64 * 0.014,
            wz as f64 * 0.014,
            3,
        )
        .max(0.0);
        let raw_y = -52.0 + continental * 18.0 + detail * 6.0 + ridge * ridge * 22.0;
        let blended = lerp(-60.0, raw_y, blend);
        (blended.round() as i32).clamp(-62, -20)
    }

    pub fn generate_chunk(&self, pos: ChunkPos) -> WorldChunk {
        let mut chunk = WorldChunk::new_empty(pos);

        let bedrock = generated_blocks::default_state::BEDROCK;
        let deepslate = generated_blocks::default_state::DEEPSLATE;
        let stone = generated_blocks::default_state::STONE;
        let dirt = generated_blocks::default_state::DIRT;
        let grass = generated_blocks::default_state::GRASS_BLOCK;
        let sand = generated_blocks::default_state::SAND;
        let sandstone =
            generated_blocks::default_state_by_name("minecraft:sandstone").unwrap_or(sand);
        let snow_block =
            generated_blocks::default_state_by_name("minecraft:snow_block").unwrap_or(grass);
        let ice = generated_blocks::default_state_by_name("minecraft:ice")
            .unwrap_or(generated_blocks::default_state::WATER);
        let cactus = generated_blocks::default_state_by_name("minecraft:cactus")
            .unwrap_or(generated_blocks::default_state::OAK_LOG);
        let water = generated_blocks::default_state::WATER;
        let cobble = generated_blocks::default_state::COBBLESTONE;
        let planks = generated_blocks::default_state::OAK_PLANKS;
        let oak_log = generated_blocks::default_state::OAK_LOG;
        let oak_leaves = generated_blocks::default_state::OAK_LEAVES;
        let birch_log =
            generated_blocks::default_state_by_name("minecraft:birch_log").unwrap_or(oak_log);
        let birch_leaves =
            generated_blocks::default_state_by_name("minecraft:birch_leaves").unwrap_or(oak_leaves);
        let spruce_log =
            generated_blocks::default_state_by_name("minecraft:spruce_log").unwrap_or(oak_log);
        let spruce_leaves = generated_blocks::default_state_by_name("minecraft:spruce_leaves")
            .unwrap_or(oak_leaves);
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
        let mut biomes = [SurfaceBiome::Plains; 256];

        for lz in 0..16 {
            let wz = pos.z * 16 + lz as i32;
            for lx in 0..16 {
                let wx = pos.x * 16 + lx as i32;
                let top_y = self.surface_height(wx, wz);
                let biome = self.biome_at(wx, wz, top_y);
                heights[lz * 16 + lx] = top_y;
                biomes[lz * 16 + lx] = biome;

                chunk.set_world_block(lx, -64, lz, bedrock);

                let is_beach = top_y <= SEA_LEVEL + 1 && biome != SurfaceBiome::SnowyTaiga;
                for y in -63..=top_y {
                    if y <= top_y - 5 && y > -63 {
                        let cave_n = value_noise_3d(
                            self.seed ^ 0xCAFE_BABE,
                            wx as f64 * 0.07,
                            y as f64 * 0.09,
                            wz as f64 * 0.07,
                        );
                        if cave_n > 0.56 {
                            if y == top_y - 5 && hash3(self.seed ^ 0x777, wx, y, wz) > 0.92 {
                                chunk.set_world_block(lx, y, lz, glowstone);
                            }
                            continue;
                        }
                    }

                    let block = if y == top_y {
                        if is_beach {
                            sand
                        } else {
                            match biome {
                                SurfaceBiome::Desert => sand,
                                SurfaceBiome::SnowyTaiga => snow_block,
                                SurfaceBiome::RockyPeaks => {
                                    if hash3(self.seed ^ 0x44, wx, y, wz) > 0.4 {
                                        stone
                                    } else {
                                        cobble
                                    }
                                }
                                SurfaceBiome::Plains | SurfaceBiome::Forest => grass,
                            }
                        }
                    } else if y >= top_y - 3 {
                        if is_beach {
                            sand
                        } else {
                            match biome {
                                SurfaceBiome::Desert => sandstone,
                                SurfaceBiome::RockyPeaks => stone,
                                _ => dirt,
                            }
                        }
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
                    let dx = (wx - 8) as f64;
                    let dz = (wz - 8) as f64;
                    if dx * dx + dz * dz > 20.0 * 20.0 {
                        for wy in (top_y + 1)..=SEA_LEVEL {
                            let fluid = if wy == SEA_LEVEL && biome == SurfaceBiome::SnowyTaiga {
                                ice
                            } else {
                                water
                            };
                            chunk.set_world_block(lx, wy, lz, fluid);
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
                    chunk.set_world_block(px, y, pz, oak_log);
                }
                chunk.set_world_block(px, -54, pz, glowstone);
            }
        } else {
            // Rare procedural surface structures (Watchtower cabins & Desert wells)
            let center_y = heights[7 * 16 + 7];
            let center_biome = biomes[7 * 16 + 7];
            let struct_roll = hash3(self.seed ^ 0x5720_C700, pos.x, 0, pos.z);
            if center_y > SEA_LEVEL + 2
                && (pos.x.abs() > 2 || pos.z.abs() > 2)
                && struct_roll > 0.94
            {
                if center_biome == SurfaceBiome::Desert {
                    place_desert_well(&mut chunk, center_y);
                } else if center_biome == SurfaceBiome::Plains {
                    place_watchtower_ruin(&mut chunk, center_y);
                }
                return chunk;
            }

            // Biome-specific vegetation (Oak, Birch, Conical Spruce, and Desert Cacti)
            for lz in 3..13 {
                for lx in 3..13 {
                    let wx = pos.x * 16 + lx as i32;
                    let wz = pos.z * 16 + lz as i32;
                    let top_y = heights[lz * 16 + lx];
                    let biome = biomes[lz * 16 + lx];
                    if top_y <= SEA_LEVEL + 2 {
                        continue;
                    }
                    let roll = hash3(self.seed ^ 0x1234_5678, wx, top_y, wz);
                    match biome {
                        SurfaceBiome::Desert => {
                            if roll > 0.978 {
                                let ch = 2 + ((wx.abs() + wz.abs()) % 2);
                                for cy in 1..=ch {
                                    chunk.set_world_block(lx, top_y + cy, lz, cactus);
                                }
                            }
                        }
                        SurfaceBiome::SnowyTaiga => {
                            if roll > 0.955 {
                                let trunk_h = 5 + ((wx.abs() + wz.abs()) % 2);
                                for ty in 1..=trunk_h {
                                    chunk.set_world_block(lx, top_y + ty, lz, spruce_log);
                                }
                                for (layer, cy) in ((top_y + 2)..=(top_y + trunk_h + 1)).enumerate()
                                {
                                    let rad: i32 = if layer % 2 == 0 && cy <= top_y + trunk_h {
                                        2
                                    } else {
                                        1
                                    };
                                    for dz in -rad..=rad {
                                        for dx in -rad..=rad {
                                            if dx == 0 && dz == 0 && cy <= top_y + trunk_h {
                                                continue;
                                            }
                                            if dx.abs() == rad && dz.abs() == rad {
                                                continue;
                                            }
                                            chunk.set_world_block(
                                                (lx as i32 + dx) as usize,
                                                cy,
                                                (lz as i32 + dz) as usize,
                                                spruce_leaves,
                                            );
                                        }
                                    }
                                }
                            }
                        }
                        SurfaceBiome::Forest | SurfaceBiome::Plains => {
                            let threshold = if biome == SurfaceBiome::Forest {
                                0.935
                            } else {
                                0.972
                            };
                            if roll > threshold {
                                let is_birch = biome == SurfaceBiome::Forest
                                    && hash3(self.seed ^ 0xB12C, wx, 0, wz) > 0.25;
                                let (t_log, t_leaves) = if is_birch {
                                    (birch_log, birch_leaves)
                                } else {
                                    (oak_log, oak_leaves)
                                };
                                let trunk_h = 4 + ((wx.abs() + wz.abs()) % 2);
                                for ty in 1..=trunk_h {
                                    chunk.set_world_block(lx, top_y + ty, lz, t_log);
                                }
                                let crown_base = top_y + trunk_h - 1;
                                for cy in crown_base..=(crown_base + 2) {
                                    let rad: i32 = if cy == crown_base + 2 { 1 } else { 2 };
                                    for dz in -rad..=rad {
                                        for dx in -rad..=rad {
                                            if dx == 0 && dz == 0 && cy <= top_y + trunk_h {
                                                continue;
                                            }
                                            if dx.abs() == rad && dz.abs() == rad && cy > crown_base
                                            {
                                                continue;
                                            }
                                            chunk.set_world_block(
                                                (lx as i32 + dx) as usize,
                                                cy,
                                                (lz as i32 + dz) as usize,
                                                t_leaves,
                                            );
                                        }
                                    }
                                }
                            }
                        }
                        SurfaceBiome::RockyPeaks => {}
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
            ChunkPos { x: 4, z: 4 },
            ChunkPos { x: -5, z: 6 },
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
    fn procedural_terrain_generates_multiple_biomes_and_keeps_spawn_plaza() {
        let terrain_gen = TerrainGenerator::new(0x501A_2A26);
        assert_eq!(terrain_gen.surface_height(8, 8), -60);

        let mut seen_non_plains = false;
        for z in (-400..400).step_by(16) {
            for x in (-400..400).step_by(16) {
                let h = terrain_gen.surface_height(x, z);
                if terrain_gen.biome_at(x, z, h) != SurfaceBiome::Plains {
                    seen_non_plains = true;
                }
            }
        }
        assert!(
            seen_non_plains,
            "expected multiple climate biomes across world"
        );

        let spawn_chunk = terrain_gen.generate_chunk(ChunkPos { x: 0, z: 0 });
        assert_eq!(
            spawn_chunk.get_world_block(7, -59, 7),
            generated_blocks::default_state::OAK_PLANKS
        );
    }
}
