//! Configuration and Play state packets for Minecraft: Java Edition 26.3 (protocol 777).

use crate::MINECRAFT_VERSION;
use crate::buf::{PacketReader, PacketWriter};
use crate::error::ProtocolResult;
use crate::nbt::{NbtTag, write_network_nbt};

/// Known resource/data pack identifier negotiated during `Configuration` (`KnownPack.java`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownPack {
    pub namespace: String,
    pub id: String,
    pub version: String,
}

impl KnownPack {
    pub fn vanilla_core() -> Self {
        Self {
            namespace: "minecraft".into(),
            id: "core".into(),
            version: MINECRAFT_VERSION.into(),
        }
    }
}

/// Encodes `ClientboundCustomPayloadPacket` (`minecraft:brand`).
pub fn encode_brand_payload(brand: &str) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.write_string("minecraft:brand");
    w.write_string(brand);
    w.into_inner()
}

/// Encodes `ClientboundUpdateEnabledFeaturesPacket`.
pub fn encode_enabled_features(features: &[&str]) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.write_varint(features.len() as i32);
    for f in features {
        w.write_string(f);
    }
    w.into_inner()
}

/// Encodes `ClientboundSelectKnownPacks`.
pub fn encode_select_known_packs(packs: &[KnownPack]) -> Vec<u8> {
    let mut w = PacketWriter::new();
    w.write_varint(packs.len() as i32);
    for p in packs {
        w.write_string(&p.namespace);
        w.write_string(&p.id);
        w.write_string(&p.version);
    }
    w.into_inner()
}

/// Decodes `ServerboundSelectKnownPacks`.
pub fn decode_select_known_packs(r: &mut PacketReader<'_>) -> ProtocolResult<Vec<KnownPack>> {
    let count = r.read_usize_bounded(64)?;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        out.push(KnownPack {
            namespace: r.read_string(256)?,
            id: r.read_string(256)?,
            version: r.read_string(256)?,
        });
    }
    Ok(out)
}

/// Encodes `ClientboundRegistryDataPacket` referencing entries from a mutually known vanilla pack
/// (`Optional<Tag>` = `false` for each entry).
pub fn encode_registry_data_known(registry_id: &str, entry_ids: &[&str]) -> Vec<u8> {
    let mut w = PacketWriter::with_capacity(64 + entry_ids.len() * 24);
    w.write_string(registry_id);
    w.write_varint(entry_ids.len() as i32);
    for id in entry_ids {
        w.write_string(id);
        w.write_bool(false); // Optional<NbtTag> empty -> client loads from known vanilla pack
    }
    w.into_inner()
}

/// Encodes `ClientboundUpdateTagsPacket` from generated registry numeric tag tables.
pub fn encode_update_tags(registries: crate::generated_registries::NetworkTagTable) -> Vec<u8> {
    let mut w = PacketWriter::with_capacity(8192);
    w.write_varint(registries.len() as i32);
    for (reg_id, tags) in registries {
        w.write_string(reg_id);
        w.write_varint(tags.len() as i32);
        for (tag_id, ids) in *tags {
            w.write_string(tag_id);
            w.write_varint(ids.len() as i32);
            for &id in *ids {
                w.write_varint(id);
            }
        }
    }
    w.into_inner()
}

/// Encodes `ClientboundDisconnectPacket` in Configuration / Play states (Network NBT Component).
pub fn encode_nbt_disconnect(reason: &str) -> Vec<u8> {
    let tag = NbtTag::Compound(vec![("text".into(), NbtTag::String(reason.to_owned()))]);
    let mut w = PacketWriter::new();
    write_network_nbt(&mut w, &tag);
    w.into_inner()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameMode {
    Survival = 0,
    Creative = 1,
    Adventure = 2,
    Spectator = 3,
}

impl GameMode {
    pub fn from_name(name: &str) -> Self {
        match name.trim().to_ascii_lowercase().as_str() {
            "creative" | "1" => Self::Creative,
            "adventure" | "2" => Self::Adventure,
            "spectator" | "3" => Self::Spectator,
            _ => Self::Survival,
        }
    }
}

/// `ClientboundLoginPacket` (`minecraft:login`, Play state) verified against MC `26.3` bytecode.
#[derive(Debug, Clone)]
pub struct PlayLoginS2c {
    pub entity_id: i32,
    pub hardcore: bool,
    pub dimension_names: Vec<String>,
    pub max_players: i32,
    pub view_distance: i32,
    pub simulation_distance: i32,
    pub reduced_debug_info: bool,
    pub show_death_screen: bool,
    pub do_limited_crafting: bool,
    pub dimension_type_id: i32,
    pub dimension_name: String,
    pub hashed_seed: i64,
    pub game_mode: GameMode,
    pub is_debug: bool,
    pub is_flat: bool,
    pub portal_cooldown: i32,
    pub sea_level: i32,
    pub online_mode: bool,
    pub enforces_secure_chat: bool,
}

impl PlayLoginS2c {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = PacketWriter::with_capacity(128);
        w.write_i32(self.entity_id);
        w.write_bool(self.hardcore);
        w.write_varint(self.dimension_names.len() as i32);
        for dim in &self.dimension_names {
            w.write_string(dim);
        }
        w.write_varint(self.max_players);
        w.write_varint(self.view_distance);
        w.write_varint(self.simulation_distance);
        w.write_bool(self.reduced_debug_info);
        w.write_bool(self.show_death_screen);
        w.write_bool(self.do_limited_crafting);

        // CommonPlayerSpawnInfo
        w.write_varint(self.dimension_type_id); // ByteBufCodecs.holderRegistry(DIMENSION_TYPE) -> VarInt registry index
        w.write_string(&self.dimension_name);
        w.write_i64(self.hashed_seed);
        w.write_u8(self.game_mode as u8);
        w.write_i8(-1); // previousGameType = none (-1)
        w.write_bool(self.is_debug);
        w.write_bool(self.is_flat);
        w.write_bool(false); // lastDeathLocation Optional = empty
        w.write_varint(self.portal_cooldown);
        w.write_varint(self.sea_level);

        // Trailing booleans in MC 26.3 ClientboundLoginPacket
        w.write_bool(self.online_mode);
        w.write_bool(self.enforces_secure_chat);
        w.into_inner()
    }
}

pub const GAME_EVENT_START_WAITING_CHUNKS: u8 = 13;

pub fn encode_game_event(event: u8, param: f32) -> Vec<u8> {
    let mut w = PacketWriter::with_capacity(5);
    w.write_u8(event);
    w.write_f32(param);
    w.into_inner()
}

pub fn encode_set_chunk_cache_center(chunk_x: i32, chunk_z: i32) -> Vec<u8> {
    let mut w = PacketWriter::with_capacity(8);
    w.write_varint(chunk_x);
    w.write_varint(chunk_z);
    w.into_inner()
}

/// `ClientboundPlayerPositionPacket` (`minecraft:player_position`) in MC `26.3`.
pub fn encode_player_position(
    teleport_id: i32,
    pos: [f64; 3],
    yaw_deg: f32,
    pitch_deg: f32,
) -> Vec<u8> {
    let mut w = PacketWriter::with_capacity(64);
    w.write_varint(teleport_id);
    // PositionMoveRotation: position Vec3(f64x3), deltaMovement Vec3(f64x3), yRot f32, xRot f32
    w.write_f64(pos[0]);
    w.write_f64(pos[1]);
    w.write_f64(pos[2]);
    w.write_f64(0.0);
    w.write_f64(0.0);
    w.write_f64(0.0);
    w.write_f32(yaw_deg);
    w.write_f32(pitch_deg);
    // Relative.SET_STREAM_CODEC: i32 bitmask (0 = all absolute coordinates)
    w.write_i32(0);
    w.into_inner()
}

/// Decodes `ServerboundAcceptTeleportationPacket` in MC `26.3`.
#[derive(Debug, Clone, Copy)]
pub struct AcceptTeleportC2s {
    pub teleport_id: i32,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub yaw: f32,
    pub pitch: f32,
}

impl AcceptTeleportC2s {
    pub fn decode(r: &mut PacketReader<'_>) -> ProtocolResult<Self> {
        Ok(Self {
            teleport_id: r.read_varint()?,
            x: r.read_f64()?,
            y: r.read_f64()?,
            z: r.read_f64()?,
            yaw: r.read_f32()?,
            pitch: r.read_f32()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_packs_roundtrip() {
        let packs = vec![KnownPack::vanilla_core()];
        let bytes = encode_select_known_packs(&packs);
        let mut r = PacketReader::new(&bytes);
        assert_eq!(decode_select_known_packs(&mut r).expect("decode"), packs);
    }
}
