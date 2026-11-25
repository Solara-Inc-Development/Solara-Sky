//! Multiplayer player list (`PlayerInfo`) and entity spawn/move/remove packets for MC 26.3 (777).

use crate::buf::{PacketReader, PacketWriter};
use crate::config_play::GameMode;
use crate::error::ProtocolResult;
use crate::packets::ProfileProperty;
use uuid::Uuid;

/// Official `minecraft:player` protocol ID in `registries.json` (`minecraft:entity_type`) for MC 26.3.
pub const ENTITY_TYPE_PLAYER: i32 = 159;

/// Action bitmask for `ClientboundPlayerInfoUpdatePacket` (`ADD_PLAYER | UPDATE_GAME_MODE | UPDATE_LISTED | UPDATE_LATENCY`).
const PLAYER_INFO_ADD_MASK: u8 = 0b0001_1101;

#[derive(Debug, Clone)]
pub struct PlayerInfoEntry {
    pub uuid: Uuid,
    pub username: String,
    pub properties: Vec<ProfileProperty>,
    pub game_mode: GameMode,
    pub latency_ms: i32,
}

#[inline]
pub fn deg_to_angle_byte(deg: f32) -> i8 {
    ((deg.rem_euclid(360.0) * 256.0 / 360.0).floor() as i32) as i8
}

/// Encodes `ClientboundPlayerInfoUpdatePacket` adding one or more players to the tab list & skin cache.
pub fn encode_player_info_add(entries: &[PlayerInfoEntry]) -> Vec<u8> {
    let mut w = PacketWriter::with_capacity(64 * entries.len());
    w.write_u8(PLAYER_INFO_ADD_MASK);
    w.write_varint(entries.len() as i32);
    for e in entries {
        w.write_uuid(e.uuid);
        // Bit 0: ADD_PLAYER
        w.write_string(&e.username);
        w.write_varint(e.properties.len() as i32);
        for p in &e.properties {
            w.write_string(&p.name);
            w.write_string(&p.value);
            match &p.signature {
                Some(sig) => {
                    w.write_bool(true);
                    w.write_string(sig);
                }
                None => w.write_bool(false),
            }
        }
        // Bit 2: UPDATE_GAME_MODE
        w.write_varint(e.game_mode as i32);
        // Bit 3: UPDATE_LISTED
        w.write_bool(true);
        // Bit 4: UPDATE_LATENCY
        w.write_varint(e.latency_ms);
    }
    w.into_inner()
}

/// Encodes `ClientboundPlayerInfoRemovePacket`.
pub fn encode_player_info_remove(uuids: &[Uuid]) -> Vec<u8> {
    let mut w = PacketWriter::with_capacity(4 + uuids.len() * 16);
    w.write_varint(uuids.len() as i32);
    for &u in uuids {
        w.write_uuid(u);
    }
    w.into_inner()
}

/// Encodes `ClientboundAddEntityPacket` (`minecraft:add_entity`) for a visible remote player (`type = 159`).
pub fn encode_add_player_entity(
    entity_id: i32,
    uuid: Uuid,
    pos: [f64; 3],
    yaw_deg: f32,
    pitch_deg: f32,
) -> Vec<u8> {
    let mut w = PacketWriter::with_capacity(64);
    w.write_varint(entity_id);
    w.write_uuid(uuid);
    w.write_varint(ENTITY_TYPE_PLAYER);
    w.write_f64(pos[0]);
    w.write_f64(pos[1]);
    w.write_f64(pos[2]);
    // Vec3.LP_STREAM_CODEC: zero velocity is encoded as single 0x00 byte in MC 26.3
    w.write_u8(0);
    w.write_i8(deg_to_angle_byte(pitch_deg));
    let yaw_b = deg_to_angle_byte(yaw_deg);
    w.write_i8(yaw_b);
    w.write_i8(yaw_b); // yHeadRot
    w.write_varint(0); // data
    w.into_inner()
}

/// Decodes `(entity_id, uuid, entity_type, pos)` from `ClientboundAddEntityPacket`.
pub fn decode_add_entity(r: &mut PacketReader<'_>) -> ProtocolResult<(i32, Uuid, i32, [f64; 3])> {
    let id = r.read_varint()?;
    let uuid = r.read_uuid()?;
    let entity_type = r.read_varint()?;
    let x = r.read_f64()?;
    let y = r.read_f64()?;
    let z = r.read_f64()?;
    Ok((id, uuid, entity_type, [x, y, z]))
}

/// Encodes `ClientboundTeleportEntityPacket` (`minecraft:teleport_entity`) for smooth remote movement.
pub fn encode_teleport_entity(
    entity_id: i32,
    pos: [f64; 3],
    yaw_deg: f32,
    pitch_deg: f32,
    on_ground: bool,
) -> Vec<u8> {
    let mut w = PacketWriter::with_capacity(68);
    w.write_varint(entity_id);
    w.write_f64(pos[0]);
    w.write_f64(pos[1]);
    w.write_f64(pos[2]);
    w.write_f64(0.0);
    w.write_f64(0.0);
    w.write_f64(0.0);
    w.write_f32(yaw_deg);
    w.write_f32(pitch_deg);
    w.write_i32(0); // Relative.SET_STREAM_CODEC = all absolute
    w.write_bool(on_ground);
    w.into_inner()
}

/// Encodes `ClientboundRemoveEntitiesPacket` (`minecraft:remove_entities`).
pub fn encode_remove_entities(entity_ids: &[i32]) -> Vec<u8> {
    let mut w = PacketWriter::with_capacity(4 + entity_ids.len() * 5);
    w.write_varint(entity_ids.len() as i32);
    for &id in entity_ids {
        w.write_varint(id);
    }
    w.into_inner()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_player_entity_roundtrips_header() {
        let uid = Uuid::new_v4();
        let bytes = encode_add_player_entity(42, uid, [8.0, -58.0, 8.0], 90.0, 0.0);
        let mut r = PacketReader::new(&bytes);
        let (id, parsed_uid, etype, pos) = decode_add_entity(&mut r).expect("decode");
        assert_eq!(id, 42);
        assert_eq!(parsed_uid, uid);
        assert_eq!(etype, ENTITY_TYPE_PLAYER);
        assert_eq!(pos, [8.0, -58.0, 8.0]);
    }
}
