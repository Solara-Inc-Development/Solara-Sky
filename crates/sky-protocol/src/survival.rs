//! Bytecode-verified Minecraft 26.3 (protocol 777) Survival, Combat, Respawn, Sound,
//! and Command Auto-Completion packet codecs.

use crate::buf::{PacketReader, PacketWriter};
use crate::config_play::GameMode;
use crate::error::ProtocolResult;

pub const CLIENT_CMD_PERFORM_RESPAWN: i32 = 0;

/// Encodes `ClientboundSetHealthPacket` (`health: f32, food: VarInt, saturation: f32`).
pub fn encode_set_health(health: f32, food: i32, saturation: f32) -> Vec<u8> {
    let mut w = PacketWriter::with_capacity(14);
    w.write_f32(health);
    w.write_varint(food);
    w.write_f32(saturation);
    w.into_inner()
}

/// Encodes `ClientboundHurtAnimationPacket` (`id: VarInt, yaw: f32`).
pub fn encode_hurt_animation(entity_id: i32, hurt_yaw_deg: f32) -> Vec<u8> {
    let mut w = PacketWriter::with_capacity(10);
    w.write_varint(entity_id);
    w.write_f32(hurt_yaw_deg);
    w.into_inner()
}

/// Decodes `ServerboundAttackPacket` (`entityId: VarInt` in MC 26.3).
pub fn decode_attack_entity(r: &mut PacketReader<'_>) -> ProtocolResult<i32> {
    r.read_varint()
}

/// Decodes `ServerboundClientCommandPacket` (`action: VarInt` -> `0 = PERFORM_RESPAWN`).
pub fn decode_client_command(r: &mut PacketReader<'_>) -> ProtocolResult<i32> {
    r.read_varint()
}

/// Encodes `ClientboundRespawnPacket` (`CommonPlayerSpawnInfo` + `u8 dataToKeep`).
pub fn encode_respawn(
    dimension_type_id: i32,
    dimension_name: &str,
    hashed_seed: i64,
    game_mode: GameMode,
    sea_level: i32,
    data_to_keep: u8,
) -> Vec<u8> {
    let mut w = PacketWriter::with_capacity(64);
    w.write_varint(dimension_type_id);
    w.write_string(dimension_name);
    w.write_i64(hashed_seed);
    w.write_u8(game_mode as u8);
    w.write_i8(-1); // previousGameType = none
    w.write_bool(false); // isDebug
    w.write_bool(false); // isFlat
    w.write_bool(false); // lastDeathLocation = empty
    w.write_varint(0); // portalCooldown
    w.write_varint(sea_level);
    w.write_u8(data_to_keep);
    w.into_inner()
}

/// Decodes `ServerboundCommandSuggestionPacket` (`id: VarInt, command: String`).
pub fn decode_command_suggestion(r: &mut PacketReader<'_>) -> ProtocolResult<(i32, String)> {
    let tx_id = r.read_varint()?;
    let command = r.read_string(32500)?;
    Ok((tx_id, command))
}

/// Encodes `ClientboundCommandSuggestionsPacket` (`id, start, length, List<Entry>`).
pub fn encode_command_suggestions(
    tx_id: i32,
    start: usize,
    length: usize,
    suggestions: &[String],
) -> Vec<u8> {
    let mut w = PacketWriter::with_capacity(16 + suggestions.len() * 16);
    w.write_varint(tx_id);
    w.write_varint(start as i32);
    w.write_varint(length as i32);
    w.write_varint(suggestions.len() as i32);
    for s in suggestions {
        w.write_string(s);
        w.write_bool(false); // Optional<Component> tooltip = empty
    }
    w.into_inner()
}

/// Encodes `ClientboundSoundPacket` (`Holder<SoundEvent>` registry ID + 1, source, fixed-point pos * 8, vol, pitch, seed).
pub fn encode_sound_effect(
    sound_registry_id: u32,
    source_category: i32,
    pos: [f64; 3],
    volume: f32,
    pitch: f32,
    seed: i64,
) -> Vec<u8> {
    let mut w = PacketWriter::with_capacity(36);
    w.write_varint((sound_registry_id + 1) as i32);
    w.write_varint(source_category);
    w.write_i32((pos[0] * 8.0) as i32);
    w.write_i32((pos[1] * 8.0) as i32);
    w.write_i32((pos[2] * 8.0) as i32);
    w.write_f32(volume);
    w.write_f32(pitch);
    w.write_i64(seed);
    w.into_inner()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attack_and_command_suggestions_roundtrip() {
        let mut w = PacketWriter::new();
        w.write_varint(77);
        let mut r = PacketReader::new(w.as_slice());
        assert_eq!(decode_attack_entity(&mut r).expect("attack"), 77);

        let pkt = encode_command_suggestions(5, 1, 2, &["gamemode".into(), "give".into()]);
        let mut sr = PacketReader::new(&pkt);
        assert_eq!(sr.read_varint().unwrap(), 5);
        assert_eq!(sr.read_varint().unwrap(), 1);
        assert_eq!(sr.read_varint().unwrap(), 2);
        assert_eq!(sr.read_varint().unwrap(), 2);
        assert_eq!(sr.read_string(64).unwrap(), "gamemode");
    }
}
