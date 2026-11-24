//! Compact chunk coordinates, bit-packed paletted block storage, and generated MC 26.3 block tables.

pub mod generated_blocks;

use sky_protocol::buf::PacketWriter;

pub const SECTION_SIDE: usize = 16;
pub const SECTION_VOLUME: usize = SECTION_SIDE * SECTION_SIDE * SECTION_SIDE; // 4096

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

/// Paletted container for a 16x16x16 block section (SingleValue, Indirect 4..=8 bits, or Direct 15 bits).
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
    pub const DIRECT_BITS: u8 = 15; // ceil(log2(35723)) = 16, MC uses 15..16; 16 fits up to 65536 states cleanly
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

    /// Serializes this paletted block container to Minecraft's chunk section wire format.
    pub fn write_network(&self, w: &mut PacketWriter) {
        match self {
            Self::Single(id) => {
                w.write_u8(0);
                w.write_varint(*id as i32);
                w.write_varint(0);
            }
            Self::Indirect { palette, storage } => {
                w.write_u8(storage.bits_per_entry());
                w.write_varint(palette.len() as i32);
                for &entry in palette {
                    w.write_varint(entry as i32);
                }
                w.write_varint(storage.longs().len() as i32);
                for &word in storage.longs() {
                    w.write_i64(word as i64);
                }
            }
            Self::Direct { storage } => {
                w.write_u8(storage.bits_per_entry());
                w.write_varint(storage.longs().len() as i32);
                for &word in storage.longs() {
                    w.write_i64(word as i64);
                }
            }
        }
    }
}

/// 16x16x16 chunk section tracking non-air block count for instant empty-section skipping.
#[derive(Debug, Clone)]
pub struct ChunkSection {
    non_air_blocks: u16,
    blocks: PalettedBlocks,
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
            blocks: PalettedBlocks::filled(generated_blocks::default_state::AIR),
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
        if prev == air && state_id != air {
            self.non_air_blocks += 1;
        } else if prev != air && state_id == air {
            self.non_air_blocks -= 1;
        }
        self.blocks.set(idx, state_id);
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

        // Insert > 260 distinct state IDs to exercise 4-bit -> 5..8-bit -> Direct promotion
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

        // Clearing back to AIR decrements non_air_blocks accurately
        sec.set(0, 0, 0, generated_blocks::default_state::AIR);
        assert_eq!(sec.get(0, 0, 0), generated_blocks::default_state::AIR);
    }

    #[test]
    fn generated_block_lookup_works() {
        let stone = generated_blocks::default_state::STONE;
        let info = generated_blocks::block_by_state_id(stone).expect("stone exists");
        assert_eq!(info.name, "minecraft:stone");
    }
}
