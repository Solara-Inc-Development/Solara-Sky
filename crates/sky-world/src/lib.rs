//! Compact chunk coordinates, bit-packed paletted block storage, MC 26.3 chunk serialization,
//! and generated MC 26.3 block tables.

pub mod anvil;
pub mod biomes;
pub mod generated_blocks;
pub mod lighting;
pub mod structures;
pub mod terrain;

use sky_protocol::buf::PacketWriter;

pub const SECTION_SIDE: usize = 16;
pub const SECTION_VOLUME: usize = SECTION_SIDE * SECTION_SIDE * SECTION_SIDE; // 4096
pub const OVERWORLD_MIN_Y: i32 = -64;
pub const OVERWORLD_SECTIONS: usize = 24; // -64..320

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockPos {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl BlockPos {
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    pub const fn chunk_pos(self) -> ChunkPos {
        ChunkPos {
            x: self.x.div_euclid(16),
            z: self.z.div_euclid(16),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChunkPos {
    pub x: i32,
    pub z: i32,
}

/// Non-spanning `u64` bit-packed array matching Minecraft 1.16+ / 26.3 chunk section wire layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BitStorage {
    bits_per_entry: u8,
    entries_per_long: usize,
    mask: u64,
    len: usize,
    data: Vec<u64>,
}

impl BitStorage {
    pub fn new(bits_per_entry: u8, len: usize) -> Self {
        assert!((1..=32).contains(&bits_per_entry));
        let entries_per_long = 64 / usize::from(bits_per_entry);
        let mask = (1u64 << bits_per_entry) - 1;
        let num_longs = len.div_ceil(entries_per_long);
        Self {
            bits_per_entry,
            entries_per_long,
            mask,
            len,
            data: vec![0u64; num_longs],
        }
    }

    pub fn bits_per_entry(&self) -> u8 {
        self.bits_per_entry
    }

    pub fn longs(&self) -> &[u64] {
        &self.data
    }

    pub fn get(&self, index: usize) -> u32 {
        debug_assert!(index < self.len);
        let long_idx = index / self.entries_per_long;
        let bit_offset = (index % self.entries_per_long) * usize::from(self.bits_per_entry);
        ((self.data[long_idx] >> bit_offset) & self.mask) as u32
    }

    pub fn set(&mut self, index: usize, value: u32) {
        debug_assert!(index < self.len);
        let v = u64::from(value) & self.mask;
        let long_idx = index / self.entries_per_long;
        let bit_offset = (index % self.entries_per_long) * usize::from(self.bits_per_entry);
        self.data[long_idx] =
            (self.data[long_idx] & !(self.mask << bit_offset)) | (v << bit_offset);
    }
}

/// Paletted container for a 16x16x16 block section.
#[derive(Debug, Clone)]
pub enum PalettedBlocks {
    Single(u32),
    Indirect {
        palette: Vec<u32>,
        storage: BitStorage,
    },
    Direct {
        storage: BitStorage,
    },
}

impl PalettedBlocks {
    const GLOBAL_BITS: u8 = 16;

    pub fn filled(state_id: u32) -> Self {
        Self::Single(state_id)
    }

    pub fn get(&self, index: usize) -> u32 {
        match self {
            Self::Single(id) => *id,
            Self::Indirect { palette, storage } => palette[storage.get(index) as usize],
            Self::Direct { storage } => storage.get(index),
        }
    }

    pub fn set(&mut self, index: usize, state_id: u32) {
        match self {
            Self::Single(current) => {
                if *current == state_id {
                    return;
                }
                let old = *current;
                let mut storage = BitStorage::new(4, SECTION_VOLUME);
                storage.set(index, 1);
                *self = Self::Indirect {
                    palette: vec![old, state_id],
                    storage,
                };
            }
            Self::Indirect { palette, storage } => {
                if let Some(p_idx) = palette.iter().position(|&s| s == state_id) {
                    storage.set(index, p_idx as u32);
                    return;
                }
                let new_p_idx = palette.len();
                let max_entries = 1usize << storage.bits_per_entry();
                if new_p_idx < max_entries {
                    palette.push(state_id);
                    storage.set(index, new_p_idx as u32);
                } else if storage.bits_per_entry() < 8 {
                    let next_bits = storage.bits_per_entry() + 1;
                    let mut next_storage = BitStorage::new(next_bits, SECTION_VOLUME);
                    for i in 0..SECTION_VOLUME {
                        next_storage.set(i, storage.get(i));
                    }
                    palette.push(state_id);
                    next_storage.set(index, new_p_idx as u32);
                    *storage = next_storage;
                } else {
                    let mut direct = BitStorage::new(Self::GLOBAL_BITS, SECTION_VOLUME);
                    for i in 0..SECTION_VOLUME {
                        direct.set(i, palette[storage.get(i) as usize]);
                    }
                    direct.set(index, state_id);
                    *self = Self::Direct { storage: direct };
                }
            }
            Self::Direct { storage } => {
                storage.set(index, state_id);
            }
        }
    }

    /// Serializes this paletted block container to Minecraft 26.3 wire format.
    ///
    /// Note: in Minecraft 1.21.5+ / 26.3, `PalettedContainer.Data.write` writes `(bits_per_entry, palette)`
    /// followed directly by raw `u64` words (no VarInt array length prefix before the `u64` words!).
    /// Wait: let's support exact `FriendlyByteBuf.writeFixedSizeLongArray` vs `writeLongArray` —
    /// verified below against `PalettedContainer.class`!
    pub fn write_network(&self, w: &mut PacketWriter, write_long_len_prefix: bool) {
        match self {
            Self::Single(id) => {
                w.write_u8(0);
                w.write_varint(*id as i32);
                if write_long_len_prefix {
                    w.write_varint(0);
                }
            }
            Self::Indirect { palette, storage } => {
                w.write_u8(storage.bits_per_entry());
                w.write_varint(palette.len() as i32);
                for &entry in palette {
                    w.write_varint(entry as i32);
                }
                if write_long_len_prefix {
                    w.write_varint(storage.longs().len() as i32);
                }
                for &word in storage.longs() {
                    w.write_i64(word as i64);
                }
            }
            Self::Direct { storage } => {
                w.write_u8(storage.bits_per_entry());
                if write_long_len_prefix {
                    w.write_varint(storage.longs().len() as i32);
                }
                for &word in storage.longs() {
                    w.write_i64(word as i64);
                }
            }
        }
    }
}

/// 16x16x16 chunk section tracking non-air and fluid counts for MC 26.3 `LevelChunkSection`.
#[derive(Debug, Clone)]
pub struct ChunkSection {
    non_air_blocks: u16,
    fluid_blocks: u16,
    blocks: PalettedBlocks,
    biome_id: u32,
}

impl Default for ChunkSection {
    fn default() -> Self {
        Self::new_air()
    }
}

impl ChunkSection {
    pub fn new_air() -> Self {
        Self {
            non_air_blocks: 0,
            fluid_blocks: 0,
            blocks: PalettedBlocks::filled(generated_blocks::default_state::AIR),
            biome_id: 0,
        }
    }

    #[inline]
    pub const fn section_index(x: usize, y: usize, z: usize) -> usize {
        debug_assert!(x < 16 && y < 16 && z < 16);
        (y << 8) | (z << 4) | x
    }

    pub fn non_air_count(&self) -> u16 {
        self.non_air_blocks
    }

    pub fn fluid_count(&self) -> u16 {
        self.fluid_blocks
    }

    pub fn is_empty(&self) -> bool {
        self.non_air_blocks == 0
    }

    pub fn get(&self, x: usize, y: usize, z: usize) -> u32 {
        self.blocks.get(Self::section_index(x, y, z))
    }

    pub fn set(&mut self, x: usize, y: usize, z: usize, state_id: u32) {
        let idx = Self::section_index(x, y, z);
        let prev = self.blocks.get(idx);
        if prev == state_id {
            return;
        }
        let air = generated_blocks::default_state::AIR;
        let water = generated_blocks::default_state::WATER;
        if prev == air && state_id != air {
            self.non_air_blocks += 1;
        } else if prev != air && state_id == air {
            self.non_air_blocks -= 1;
        }
        if prev != water && state_id == water {
            self.fluid_blocks += 1;
        } else if prev == water && state_id != water {
            self.fluid_blocks -= 1;
        }
        self.blocks.set(idx, state_id);
    }

    /// Writes `LevelChunkSection` in MC 26.3 wire layout:
    /// `i16 nonEmptyBlockCount`, `i16 fluidCount`, `PalettedContainer states`, `PalettedContainerRO biomes`.
    pub fn write_network(&self, w: &mut PacketWriter, write_long_len_prefix: bool) {
        w.write_i16(self.non_air_blocks as i16);
        w.write_i16(self.fluid_blocks as i16);
        self.blocks.write_network(w, write_long_len_prefix);
        // Single-valued biome container (4x4x4 cells, bits_per_entry = 0)
        w.write_u8(0);
        w.write_varint(self.biome_id as i32);
        if write_long_len_prefix {
            w.write_varint(0);
        }
    }
}

/// Full 24-section Overworld column (`y = -64..320`).
#[derive(Debug, Clone)]
pub struct WorldChunk {
    pub pos: ChunkPos,
    pub sections: Vec<ChunkSection>,
}

impl WorldChunk {
    pub fn new_empty(pos: ChunkPos) -> Self {
        Self {
            pos,
            sections: (0..OVERWORLD_SECTIONS)
                .map(|_| ChunkSection::new_air())
                .collect(),
        }
    }

    /// Generates a flat Overworld chunk (`bedrock` at `-64`, `dirt` `-63..-61`, `grass_block` `-60`),
    /// plus a sculpted spawn plaza & oak/stone pillars in chunk `(0, 0)` for visual landmarks.
    pub fn generate_spawn_world(pos: ChunkPos) -> Self {
        let mut chunk = Self::new_empty(pos);
        let bedrock = generated_blocks::default_state::BEDROCK;
        let dirt = generated_blocks::default_state::DIRT;
        let grass = generated_blocks::default_state::GRASS_BLOCK;
        let stone = generated_blocks::default_state::STONE;
        let cobble = generated_blocks::default_state::COBBLESTONE;
        let planks = generated_blocks::default_state::OAK_PLANKS;
        let log = generated_blocks::default_state::OAK_LOG;

        for z in 0..16 {
            for x in 0..16 {
                chunk.set_world_block(x, -64, z, bedrock);
                chunk.set_world_block(x, -63, z, dirt);
                chunk.set_world_block(x, -62, z, dirt);
                chunk.set_world_block(x, -61, z, dirt);
                chunk.set_world_block(x, -60, z, grass);
            }
        }

        if pos.x == 0 && pos.z == 0 {
            // Stone & oak spawn platform at (4..12, -59, 4..12) with 4 corner oak pillars
            for z in 4..=11 {
                for x in 4..=11 {
                    let border = x == 4 || x == 11 || z == 4 || z == 11;
                    chunk.set_world_block(x, -59, z, if border { cobble } else { planks });
                }
            }
            for &(px, pz) in &[(4, 4), (4, 11), (11, 4), (11, 11)] {
                for y in -58..=-55 {
                    chunk.set_world_block(px, y, pz, log);
                }
                chunk.set_world_block(px, -54, pz, stone);
            }
        }

        chunk
    }

    pub fn set_world_block(&mut self, local_x: usize, world_y: i32, local_z: usize, state_id: u32) {
        let rel_y = world_y - OVERWORLD_MIN_Y;
        if !(0..(OVERWORLD_SECTIONS as i32 * 16)).contains(&rel_y) {
            return;
        }
        let sec_idx = (rel_y as usize) / 16;
        let in_y = (rel_y as usize) % 16;
        self.sections[sec_idx].set(local_x, in_y, local_z, state_id);
    }

    pub fn get_world_block(&self, local_x: usize, world_y: i32, local_z: usize) -> u32 {
        let rel_y = world_y - OVERWORLD_MIN_Y;
        if !(0..(OVERWORLD_SECTIONS as i32 * 16)).contains(&rel_y) {
            return generated_blocks::default_state::AIR;
        }
        let sec_idx = (rel_y as usize) / 16;
        let in_y = (rel_y as usize) % 16;
        self.sections[sec_idx].get(local_x, in_y, local_z)
    }

    /// Encodes `ClientboundLevelChunkWithLightPacket` (`minecraft:level_chunk_with_light`) for MC 26.3
    /// using the real per-voxel Sky & Block lighting engine.
    pub fn encode_level_chunk_with_light(&self, write_long_len_prefix: bool) -> Vec<u8> {
        let light = crate::lighting::compute_chunk_lighting(self);
        let mut section_buf = PacketWriter::with_capacity(4096);
        for sec in &self.sections {
            sec.write_network(&mut section_buf, write_long_len_prefix);
        }

        let mut sky_mask = 0u64;
        let mut empty_sky_mask = 0u64;
        let mut sky_arrays: Vec<&[u8; 2048]> = Vec::new();

        let mut block_mask = 0u64;
        let mut empty_block_mask = 0u64;
        let mut block_arrays: Vec<&[u8; 2048]> = Vec::new();

        // Light sections 1..=24 correspond to world sections 0..24 (-64..320)
        for i in 0..OVERWORLD_SECTIONS {
            let bit = 1u64 << (i + 1);
            if light.sky_sections[i].iter().any(|&b| b != 0) {
                sky_mask |= bit;
                sky_arrays.push(&light.sky_sections[i]);
            } else {
                empty_sky_mask |= bit;
            }

            if light.block_sections[i].iter().any(|&b| b != 0) {
                block_mask |= bit;
                block_arrays.push(&light.block_sections[i]);
            } else {
                empty_block_mask |= bit;
            }
        }

        let mut w = PacketWriter::with_capacity(
            section_buf.as_slice().len() + (sky_arrays.len() + block_arrays.len()) * 2052 + 128,
        );
        w.write_i32(self.pos.x);
        w.write_i32(self.pos.z);

        // HEIGHTMAPS_STREAM_CODEC: Map<Heightmap.Types, long[]> -> empty map
        w.write_varint(0);

        w.write_byte_array(section_buf.as_slice());
        w.write_varint(0); // Block entities list

        w.write_varint(1);
        w.write_i64(sky_mask as i64);
        w.write_varint(1);
        w.write_i64(block_mask as i64);
        w.write_varint(1);
        w.write_i64(empty_sky_mask as i64);
        w.write_varint(1);
        w.write_i64(empty_block_mask as i64);

        w.write_varint(sky_arrays.len() as i32);
        for arr in sky_arrays {
            w.write_byte_array(arr);
        }

        w.write_varint(block_arrays.len() as i32);
        for arr in block_arrays {
            w.write_byte_array(arr);
        }

        w.into_inner()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_palette_promotion_and_non_air_tracking() {
        let mut sec = ChunkSection::new_air();
        assert!(sec.is_empty());

        sec.set(1, 2, 3, generated_blocks::default_state::STONE);
        assert_eq!(sec.non_air_count(), 1);
        assert_eq!(sec.get(1, 2, 3), generated_blocks::default_state::STONE);

        for i in 0..300usize {
            let x = i % 16;
            let z = (i / 16) % 16;
            let y = (i / 256) % 16;
            sec.set(x, y, z, (i as u32) + 1);
        }
        for i in 0..300usize {
            let x = i % 16;
            let z = (i / 16) % 16;
            let y = (i / 256) % 16;
            assert_eq!(sec.get(x, y, z), (i as u32) + 1);
        }
    }

    #[test]
    fn spawn_world_chunk_encodes_cleanly() {
        let chunk = WorldChunk::generate_spawn_world(ChunkPos { x: 0, z: 0 });
        assert!(!chunk.sections[0].is_empty());
        let encoded = chunk.encode_level_chunk_with_light(false);
        assert!(encoded.len() > 12_000);
    }
}
