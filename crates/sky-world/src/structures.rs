//! Procedural Overworld surface structures (stone watchtower ruins & desert sandstone wells).

use crate::{WorldChunk, generated_blocks};

/// Stamps a cozy 5x5 stone & oak watchtower cabin inside chunk interior `(5..=9, 5..=9)` at `base_y`.
pub fn place_watchtower_ruin(chunk: &mut WorldChunk, base_y: i32) {
    let cobble = generated_blocks::default_state::COBBLESTONE;
    let planks = generated_blocks::default_state::OAK_PLANKS;
    let log = generated_blocks::default_state::OAK_LOG;
    let glow = generated_blocks::default_state::GLOWSTONE;
    let air = generated_blocks::default_state::AIR;

    for lz in 5..=9 {
        for lx in 5..=9 {
            chunk.set_world_block(lx, base_y, lz, planks);
            let corner = (lx == 5 || lx == 9) && (lz == 5 || lz == 9);
            let wall = lx == 5 || lx == 9 || lz == 5 || lz == 9;
            let doorway = lx == 7 && lz == 5;
            for dy in 1..=3 {
                let blk = if doorway && dy <= 2 {
                    air
                } else if corner {
                    log
                } else if wall && dy <= 2 {
                    cobble
                } else {
                    air
                };
                chunk.set_world_block(lx, base_y + dy, lz, blk);
            }
        }
    }
    chunk.set_world_block(7, base_y + 3, 7, glow);
}

/// Stamps a classic 3x3 Desert Sandstone Well with fresh water in the center!
pub fn place_desert_well(chunk: &mut WorldChunk, base_y: i32) {
    let sandstone = generated_blocks::default_state_by_name("minecraft:sandstone")
        .unwrap_or(generated_blocks::default_state::SAND);
    let water = generated_blocks::default_state::WATER;
    let glow = generated_blocks::default_state::GLOWSTONE;

    for lz in 6..=8 {
        for lx in 6..=8 {
            chunk.set_world_block(lx, base_y, lz, sandstone);
            if lx == 7 && lz == 7 {
                chunk.set_world_block(lx, base_y + 1, lz, water);
            } else {
                chunk.set_world_block(lx, base_y + 1, lz, sandstone);
            }
            if (lx == 6 || lx == 8) && (lz == 6 || lz == 8) {
                chunk.set_world_block(lx, base_y + 2, lz, sandstone);
                chunk.set_world_block(lx, base_y + 3, lz, sandstone);
            }
        }
    }
    for lz in 6..=8 {
        for lx in 6..=8 {
            chunk.set_world_block(
                lx,
                base_y + 4,
                lz,
                if lx == 7 && lz == 7 { glow } else { sandstone },
            );
        }
    }
}
