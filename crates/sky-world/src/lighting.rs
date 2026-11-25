//! Fast per-chunk Sky & Block lighting engine producing MC 26.3 4-bit nibble arrays.

use crate::{OVERWORLD_MIN_Y, OVERWORLD_SECTIONS, SECTION_VOLUME, WorldChunk, generated_blocks};
use std::collections::VecDeque;

const WORLD_HEIGHT: usize = OVERWORLD_SECTIONS * 16; // 384

/// Packed 4-bit nibble light storage (`2048` bytes per 16x16x16 section) for an entire chunk column.
#[derive(Debug, Clone)]
pub struct ChunkLighting {
    pub sky_sections: Vec<[u8; 2048]>,
    pub block_sections: Vec<[u8; 2048]>,
}

impl ChunkLighting {
    pub fn get_sky(&self, x: usize, world_y: i32, z: usize) -> u8 {
        let ry = (world_y - OVERWORLD_MIN_Y) as usize;
        let sec = ry / 16;
        let idx = ((ry % 16) << 8) | (z << 4) | x;
        let byte = self.sky_sections[sec][idx >> 1];
        if (idx & 1) == 0 {
            byte & 0x0F
        } else {
            byte >> 4
        }
    }

    pub fn get_block(&self, x: usize, world_y: i32, z: usize) -> u8 {
        let ry = (world_y - OVERWORLD_MIN_Y) as usize;
        let sec = ry / 16;
        let idx = ((ry % 16) << 8) | (z << 4) | x;
        let byte = self.block_sections[sec][idx >> 1];
        if (idx & 1) == 0 {
            byte & 0x0F
        } else {
            byte >> 4
        }
    }
}

#[inline]
fn set_nibble(sections: &mut [[u8; 2048]], ry: usize, x: usize, z: usize, val: u8) {
    let sec = ry >> 4;
    let idx = ((ry & 0xF) << 8) | (z << 4) | x;
    let slot = &mut sections[sec][idx >> 1];
    if (idx & 1) == 0 {
        *slot = (*slot & 0xF0) | (val & 0x0F);
    } else {
        *slot = (*slot & 0x0F) | ((val & 0x0F) << 4);
    }
}

#[inline]
fn get_nibble(sections: &[[u8; 2048]], ry: usize, x: usize, z: usize) -> u8 {
    let sec = ry >> 4;
    let idx = ((ry & 0xF) << 8) | (z << 4) | x;
    let byte = sections[sec][idx >> 1];
    if (idx & 1) == 0 {
        byte & 0x0F
    } else {
        byte >> 4
    }
}

#[inline]
fn light_opacity(state_id: u32) -> u8 {
    if state_id == generated_blocks::default_state::AIR
        || state_id == generated_blocks::default_state::GLASS
    {
        0
    } else if state_id == generated_blocks::default_state::OAK_LEAVES {
        1
    } else if state_id == generated_blocks::default_state::WATER {
        2
    } else {
        15
    }
}

#[inline]
fn block_emission(state_id: u32) -> u8 {
    if state_id == generated_blocks::default_state::GLOWSTONE {
        15
    } else {
        0
    }
}

fn bfs_propagate(
    chunk: &WorldChunk,
    sections: &mut [[u8; 2048]],
    mut queue: VecDeque<(u8, u16, u8, u8)>,
) {
    const DIRS: [(isize, isize, isize); 6] = [
        (-1, 0, 0),
        (1, 0, 0),
        (0, -1, 0),
        (0, 1, 0),
        (0, 0, -1),
        (0, 0, 1),
    ];

    while let Some((x, ry, z, level)) = queue.pop_front() {
        if level <= 1 {
            continue;
        }
        for (dx, dy, dz) in DIRS {
            let nx = x as isize + dx;
            let nry = ry as isize + dy;
            let nz = z as isize + dz;
            if !(0..16).contains(&nx)
                || !(0..WORLD_HEIGHT as isize).contains(&nry)
                || !(0..16).contains(&nz)
            {
                continue;
            }
            let (ux, ury, uz) = (nx as usize, nry as usize, nz as usize);
            let st = chunk.sections[ury >> 4].get(ux, ury & 0xF, uz);
            let opacity = light_opacity(st).max(1);
            if opacity >= level {
                continue;
            }
            let next_level = level - opacity;
            if next_level > get_nibble(sections, ury, ux, uz) {
                set_nibble(sections, ury, ux, uz, next_level);
                queue.push_back((ux as u8, ury as u16, uz as u8, next_level));
            }
        }
    }
}

/// Computes full 24-section Sky Light and Block Light nibble arrays for `chunk`.
pub fn compute_chunk_lighting(chunk: &WorldChunk) -> ChunkLighting {
    let mut sky_sections = vec![[0u8; SECTION_VOLUME / 2]; OVERWORLD_SECTIONS];
    let mut block_sections = vec![[0u8; SECTION_VOLUME / 2]; OVERWORLD_SECTIONS];

    // Highest non-empty section optimization: everything above `top_non_empty` is pure 15 sky light!
    let top_non_empty = chunk
        .sections
        .iter()
        .rposition(|s| !s.is_empty())
        .unwrap_or(0);

    for sec in &mut sky_sections[(top_non_empty + 1)..OVERWORLD_SECTIONS] {
        sec.fill(0xFF);
    }

    let mut sky_queue = VecDeque::with_capacity(512);
    let max_scan_ry = ((top_non_empty + 1) * 16).min(WORLD_HEIGHT) - 1;

    for z in 0..16 {
        for x in 0..16 {
            let mut current_sky = 15u8;
            for ry in (0..=max_scan_ry).rev() {
                let st = chunk.sections[ry >> 4].get(x, ry & 0xF, z);
                let op = light_opacity(st);
                if op >= 15 {
                    current_sky = 0;
                } else {
                    current_sky = current_sky.saturating_sub(op);
                }
                if current_sky > 0 {
                    set_nibble(&mut sky_sections, ry, x, z, current_sky);
                    // Seed horizontal BFS only around solid/attenuated regions near terrain
                    if current_sky < 15 || ry <= max_scan_ry {
                        sky_queue.push_back((x as u8, ry as u16, z as u8, current_sky));
                    }
                }
            }
        }
    }
    bfs_propagate(chunk, &mut sky_sections, sky_queue);

    // Block light pass from emissive blocks (e.g., Glowstone pillars & cave crystals)
    let mut block_queue = VecDeque::new();
    for (sec_idx, sec) in chunk.sections.iter().enumerate() {
        if sec.is_empty() {
            continue;
        }
        for ly in 0..16 {
            let ry = (sec_idx << 4) | ly;
            for z in 0..16 {
                for x in 0..16 {
                    let em = block_emission(sec.get(x, ly, z));
                    if em > 0 {
                        set_nibble(&mut block_sections, ry, x, z, em);
                        block_queue.push_back((x as u8, ry as u16, z as u8, em));
                    }
                }
            }
        }
    }
    if !block_queue.is_empty() {
        bfs_propagate(chunk, &mut block_sections, block_queue);
    }

    ChunkLighting {
        sky_sections,
        block_sections,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ChunkPos;

    #[test]
    fn sky_and_block_lighting_propagate_accurately() {
        let mut chunk = WorldChunk::generate_spawn_world(ChunkPos { x: 0, z: 0 });
        chunk.set_world_block(8, -58, 8, generated_blocks::default_state::GLOWSTONE);
        // Roof block at (2, -55, 2) casting shadow beneath
        chunk.set_world_block(2, -55, 2, generated_blocks::default_state::STONE);

        let light = compute_chunk_lighting(&chunk);
        // Open sky above plaza = 15
        assert_eq!(light.get_sky(8, -50, 8), 15);
        // Deep inside solid dirt at y = -62 = 0
        assert_eq!(light.get_sky(8, -62, 8), 0);
        // Under single block roof at (2, -56, 2): horizontal BFS spills sky light 14!
        assert_eq!(light.get_sky(2, -56, 2), 14);
        // Emissive glowstone at (8, -58, 8) = 15, neighbor (9, -58, 8) = 14
        assert_eq!(light.get_block(8, -58, 8), 15);
        assert_eq!(light.get_block(9, -58, 8), 14);
    }
}
