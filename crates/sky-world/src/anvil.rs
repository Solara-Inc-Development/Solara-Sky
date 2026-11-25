//! Standard Minecraft Anvil (`.mca`) region file reader and writer (`32x32` chunks per region).

use crate::{
    BitStorage, ChunkPos, ChunkSection, OVERWORLD_MIN_Y, OVERWORLD_SECTIONS, PalettedBlocks,
    SECTION_VOLUME, WorldChunk, generated_blocks,
};
use flate2::Compression;
use flate2::read::{GzDecoder, ZlibDecoder};
use flate2::write::ZlibEncoder;
use sky_protocol::buf::{PacketReader, PacketWriter};
use sky_protocol::nbt::{NbtTag, read_named_nbt, write_named_nbt};
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

const SECTOR_BYTES: usize = 4096;
const HEADER_SECTORS: usize = 2; // 4 KiB locations + 4 KiB timestamps
const COMPRESSION_GZIP: u8 = 1;
const COMPRESSION_ZLIB: u8 = 2;
const COMPRESSION_NONE: u8 = 3;
const ANVIL_DATA_VERSION: i32 = 4325;

#[derive(Debug, Error)]
pub enum AnvilError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("protocol/NBT decode error: {0}")]
    Protocol(#[from] sky_protocol::ProtocolError),
    #[error("corrupt Anvil region file: {0}")]
    CorruptRegion(String),
}

pub fn region_coords(pos: ChunkPos) -> (i32, i32) {
    (pos.x.div_euclid(32), pos.z.div_euclid(32))
}

pub fn region_file_path(region_dir: &Path, rx: i32, rz: i32) -> PathBuf {
    region_dir.join(format!("r.{rx}.{rz}.mca"))
}

fn chunk_slot_index(pos: ChunkPos) -> usize {
    let lx = pos.x.rem_euclid(32) as usize;
    let lz = pos.z.rem_euclid(32) as usize;
    lx + lz * 32
}

/// Encodes a `WorldChunk` into an uncompressed Anvil Level Root NBT compound.
pub fn chunk_to_nbt(chunk: &WorldChunk) -> NbtTag {
    let mut section_tags = Vec::new();

    for (sec_idx, sec) in chunk.sections.iter().enumerate() {
        if sec.is_empty() {
            continue;
        }
        let section_y = ((OVERWORLD_MIN_Y / 16) + sec_idx as i32) as i8;

        // Collect unique block state IDs in this section
        let mut state_to_idx: HashMap<u32, u32> = HashMap::new();
        let mut palette_states: Vec<u32> = Vec::new();
        for y in 0..16 {
            for z in 0..16 {
                for x in 0..16 {
                    let st = sec.get(x, y, z);
                    state_to_idx.entry(st).or_insert_with(|| {
                        let idx = palette_states.len() as u32;
                        palette_states.push(st);
                        idx
                    });
                }
            }
        }

        let palette_nbt: Vec<NbtTag> = palette_states
            .iter()
            .map(|&st| {
                // Store exact state_id alongside canonical block Name so multi-property blocks
                // preserve lossless state IDs while staying valid Anvil NBT!
                let name = generated_blocks::block_by_state_id(st)
                    .map(|b| b.name)
                    .unwrap_or("minecraft:air");
                NbtTag::Compound(vec![
                    ("Name".into(), NbtTag::String(name.to_owned())),
                    ("SolaraStateId".into(), NbtTag::Int(st as i32)),
                ])
            })
            .collect();

        let mut block_states_fields = vec![("palette".into(), NbtTag::List(palette_nbt))];

        if palette_states.len() > 1 {
            let bits = (usize::BITS - (palette_states.len() - 1).leading_zeros()).max(4) as u8;
            let mut storage = BitStorage::new(bits, SECTION_VOLUME);
            for i in 0..SECTION_VOLUME {
                let x = i & 0xF;
                let z = (i >> 4) & 0xF;
                let y = i >> 8;
                let st = sec.get(x, y, z);
                storage.set(i, state_to_idx[&st]);
            }
            let longs: Vec<i64> = storage.longs().iter().map(|&w| w as i64).collect();
            block_states_fields.push(("data".into(), NbtTag::LongArray(longs)));
        }

        section_tags.push(NbtTag::Compound(vec![
            ("Y".into(), NbtTag::Byte(section_y)),
            ("block_states".into(), NbtTag::Compound(block_states_fields)),
        ]));
    }

    NbtTag::Compound(vec![
        ("DataVersion".into(), NbtTag::Int(ANVIL_DATA_VERSION)),
        ("xPos".into(), NbtTag::Int(chunk.pos.x)),
        ("yPos".into(), NbtTag::Int(OVERWORLD_MIN_Y / 16)),
        ("zPos".into(), NbtTag::Int(chunk.pos.z)),
        ("Status".into(), NbtTag::String("minecraft:full".into())),
        ("sections".into(), NbtTag::List(section_tags)),
    ])
}

/// Reconstructs a `WorldChunk` from an Anvil Level Root NBT compound.
pub fn chunk_from_nbt(root: &NbtTag) -> Result<WorldChunk, AnvilError> {
    let cx = match root.get("xPos") {
        Some(NbtTag::Int(v)) => *v,
        _ => return Err(AnvilError::CorruptRegion("missing xPos Int".into())),
    };
    let cz = match root.get("zPos") {
        Some(NbtTag::Int(v)) => *v,
        _ => return Err(AnvilError::CorruptRegion("missing zPos Int".into())),
    };

    let mut chunk = WorldChunk::new_empty(ChunkPos { x: cx, z: cz });

    let Some(NbtTag::List(sections)) = root.get("sections") else {
        return Ok(chunk);
    };

    for sec_tag in sections {
        let Some(NbtTag::Byte(sec_y)) = sec_tag.get("Y") else {
            continue;
        };
        let sec_idx_i32 = i32::from(*sec_y) - (OVERWORLD_MIN_Y / 16);
        if !(0..OVERWORLD_SECTIONS as i32).contains(&sec_idx_i32) {
            continue;
        }
        let sec_idx = sec_idx_i32 as usize;

        let Some(bs_tag) = sec_tag.get("block_states") else {
            continue;
        };
        let Some(NbtTag::List(palette_list)) = bs_tag.get("palette") else {
            continue;
        };
        if palette_list.is_empty() {
            continue;
        }

        let mut palette_ids = Vec::with_capacity(palette_list.len());
        for entry in palette_list {
            if let Some(NbtTag::Int(exact_id)) = entry.get("SolaraStateId") {
                palette_ids.push(*exact_id as u32);
            } else if let Some(NbtTag::String(name)) = entry.get("Name") {
                palette_ids.push(generated_blocks::default_state_by_name(name).unwrap_or(0));
            } else {
                palette_ids.push(0);
            }
        }

        if palette_ids.len() == 1 {
            let st = palette_ids[0];
            if st != generated_blocks::default_state::AIR {
                for y in 0..16 {
                    for z in 0..16 {
                        for x in 0..16 {
                            chunk.sections[sec_idx].set(x, y, z, st);
                        }
                    }
                }
            }
            continue;
        }

        if let Some(NbtTag::LongArray(longs)) = bs_tag.get("data") {
            let bits = (usize::BITS - (palette_ids.len() - 1).leading_zeros()).max(4) as usize;
            let entries_per_long = 64 / bits;
            let mask = (1u64 << bits) - 1;
            let mut section = ChunkSection::new_air();
            for i in 0..SECTION_VOLUME {
                let long_idx = i / entries_per_long;
                let bit_off = (i % entries_per_long) * bits;
                if let Some(&word) = longs.get(long_idx) {
                    let p_idx = (((word as u64) >> bit_off) & mask) as usize;
                    let st = palette_ids.get(p_idx).copied().unwrap_or(0);
                    if st != generated_blocks::default_state::AIR {
                        section.set(i & 0xF, i >> 8, (i >> 4) & 0xF, st);
                    }
                }
            }
            let _ = PalettedBlocks::filled(0); // keep import clean
            chunk.sections[sec_idx] = section;
        }
    }

    Ok(chunk)
}

/// In-memory Anvil `.mca` region container (`32x32` compressed chunk payloads) with atomic disk save.
#[derive(Debug, Clone)]
pub struct AnvilRegion {
    compressed_chunks: Vec<Option<(u32, Vec<u8>)>>,
}

impl Default for AnvilRegion {
    fn default() -> Self {
        Self::new_empty()
    }
}

impl AnvilRegion {
    pub fn new_empty() -> Self {
        Self {
            compressed_chunks: vec![None; 1024],
        }
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, AnvilError> {
        if bytes.is_empty() {
            return Ok(Self::new_empty());
        }
        if bytes.len() < HEADER_SECTORS * SECTOR_BYTES {
            return Err(AnvilError::CorruptRegion(
                "region file shorter than 8 KiB header".into(),
            ));
        }

        let mut compressed_chunks = vec![None; 1024];
        for (slot, slot_entry) in compressed_chunks.iter_mut().enumerate() {
            let loc_off = slot * 4;
            let entry = u32::from_be_bytes([
                bytes[loc_off],
                bytes[loc_off + 1],
                bytes[loc_off + 2],
                bytes[loc_off + 3],
            ]);
            let sector_offset = (entry >> 8) as usize;
            let sector_count = (entry & 0xFF) as usize;
            if sector_offset < HEADER_SECTORS || sector_count == 0 {
                continue;
            }

            let ts_off = SECTOR_BYTES + slot * 4;
            let timestamp = u32::from_be_bytes([
                bytes[ts_off],
                bytes[ts_off + 1],
                bytes[ts_off + 2],
                bytes[ts_off + 3],
            ]);

            let byte_start = sector_offset * SECTOR_BYTES;
            let byte_limit = byte_start + sector_count * SECTOR_BYTES;
            if byte_start + 5 > bytes.len() || byte_limit > bytes.len() {
                return Err(AnvilError::CorruptRegion("sector out of bounds".into()));
            }

            let exact_len = u32::from_be_bytes([
                bytes[byte_start],
                bytes[byte_start + 1],
                bytes[byte_start + 2],
                bytes[byte_start + 3],
            ]) as usize;
            if exact_len == 0 || byte_start + 4 + exact_len > byte_limit {
                return Err(AnvilError::CorruptRegion(
                    "chunk payload exceeds sector allocation".into(),
                ));
            }
            let payload = bytes[byte_start + 4..byte_start + 4 + exact_len].to_vec();
            *slot_entry = Some((timestamp, payload));
        }

        Ok(Self { compressed_chunks })
    }

    pub fn load_or_create(path: &Path) -> Result<Self, AnvilError> {
        if !path.exists() {
            return Ok(Self::new_empty());
        }
        let raw = fs::read(path)?;
        Self::from_bytes(&raw)
    }

    pub fn put_chunk(&mut self, chunk: &WorldChunk, timestamp_secs: u32) -> Result<(), AnvilError> {
        let nbt = chunk_to_nbt(chunk);
        let mut w = PacketWriter::with_capacity(4096);
        write_named_nbt(&mut w, "", &nbt);

        let mut z = ZlibEncoder::new(Vec::new(), Compression::fast());
        z.write_all(w.as_slice())?;
        let compressed = z.finish()?;

        let mut payload = Vec::with_capacity(1 + compressed.len());
        payload.push(COMPRESSION_ZLIB);
        payload.extend_from_slice(&compressed);

        let slot = chunk_slot_index(chunk.pos);
        self.compressed_chunks[slot] = Some((timestamp_secs, payload));
        Ok(())
    }

    pub fn get_chunk(&self, pos: ChunkPos) -> Result<Option<WorldChunk>, AnvilError> {
        let slot = chunk_slot_index(pos);
        let Some((_, payload)) = &self.compressed_chunks[slot] else {
            return Ok(None);
        };
        if payload.is_empty() {
            return Ok(None);
        }

        let scheme = payload[0];
        let body = &payload[1..];
        let mut raw_nbt = Vec::new();
        match scheme {
            COMPRESSION_GZIP => {
                GzDecoder::new(body)
                    .take(16 * 1024 * 1024)
                    .read_to_end(&mut raw_nbt)?;
            }
            COMPRESSION_ZLIB => {
                ZlibDecoder::new(body)
                    .take(16 * 1024 * 1024)
                    .read_to_end(&mut raw_nbt)?;
            }
            COMPRESSION_NONE => {
                raw_nbt.extend_from_slice(body);
            }
            other => {
                return Err(AnvilError::CorruptRegion(format!(
                    "unsupported Anvil compression type {other}"
                )));
            }
        }

        let mut r = PacketReader::new(&raw_nbt);
        let (_name, tag) = read_named_nbt(&mut r)?;
        Ok(Some(chunk_from_nbt(&tag)?))
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = vec![0u8; HEADER_SECTORS * SECTOR_BYTES];
        let mut next_sector = HEADER_SECTORS;

        for (slot, maybe) in self.compressed_chunks.iter().enumerate() {
            let Some((timestamp, payload)) = maybe else {
                continue;
            };
            let total_chunk_bytes = 4 + payload.len();
            let sectors_needed = total_chunk_bytes.div_ceil(SECTOR_BYTES);
            if sectors_needed > 255 {
                continue;
            }

            let loc_entry = ((next_sector as u32) << 8) | (sectors_needed as u32);
            let loc_off = slot * 4;
            out[loc_off..loc_off + 4].copy_from_slice(&loc_entry.to_be_bytes());

            let ts_off = SECTOR_BYTES + slot * 4;
            out[ts_off..ts_off + 4].copy_from_slice(&timestamp.to_be_bytes());

            let start = out.len();
            out.resize(start + sectors_needed * SECTOR_BYTES, 0);
            out[start..start + 4].copy_from_slice(&(payload.len() as u32).to_be_bytes());
            out[start + 4..start + 4 + payload.len()].copy_from_slice(payload);

            next_sector += sectors_needed;
        }
        out
    }

    pub fn save_atomic(&self, path: &Path) -> Result<(), AnvilError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("mca.tmp");
        fs::write(&tmp, self.to_bytes())?;
        fs::rename(&tmp, path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anvil_mca_region_roundtrips_modified_chunks() {
        let pos = ChunkPos { x: 0, z: 0 };
        let mut chunk = WorldChunk::generate_spawn_world(pos);
        let glass = generated_blocks::default_state::GLASS;
        let deepslate = generated_blocks::default_state::DEEPSLATE;
        chunk.set_world_block(7, -57, 7, glass);
        chunk.set_world_block(2, 10, 14, deepslate);

        let mut region = AnvilRegion::new_empty();
        region.put_chunk(&chunk, 1_700_000_000).expect("put chunk");

        let mca_bytes = region.to_bytes();
        assert_eq!(mca_bytes.len() % SECTOR_BYTES, 0);

        let parsed = AnvilRegion::from_bytes(&mca_bytes).expect("parse mca");
        let loaded = parsed
            .get_chunk(pos)
            .expect("get chunk")
            .expect("chunk exists");

        assert_eq!(loaded.pos, pos);
        assert_eq!(loaded.sections[0].get(7, (-57 - -64) as usize, 7), glass);
        assert_eq!(
            loaded.sections[((10 - -64) / 16) as usize].get(2, ((10 - -64) % 16) as usize, 14),
            deepslate
        );
    }
}
