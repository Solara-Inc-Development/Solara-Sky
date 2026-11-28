//! Bytecode-verified Minecraft 26.3 (protocol 777) Survival, Combat, Inventory, WorldClock Time,
//! Item Pickup, Respawn, Sound, and Command Auto-Completion packet codecs.

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

/// Encodes bytecode-verified MC `26.3` `ClientboundSetExperiencePacket` (`s2c::SET_EXPERIENCE = 0x69`):
/// wire order: `(experienceProgress: f32, experienceLevel: VarInt, totalExperience: VarInt)`.
pub fn encode_set_experience(progress: f32, level: i32, total_xp: i32) -> Vec<u8> {
    let mut w = PacketWriter::with_capacity(14);
    w.write_f32(progress.clamp(0.0, 1.0));
    w.write_varint(level.max(0));
    w.write_varint(total_xp.max(0));
    w.into_inner()
}

/// Encodes bytecode-verified MC `26.3` `ClientboundUpdateMobEffectPacket` (`s2c::UPDATE_MOB_EFFECT = 0x87`):
/// `(entityId: VarInt, effect: holderRegistry VarInt 0-based, amplifier: VarInt, durationTicks: VarInt, flags: u8)`.
pub fn encode_update_mob_effect(
    entity_id: i32,
    effect_id: u32,
    amplifier: u8,
    duration_ticks: i32,
) -> Vec<u8> {
    let mut w = PacketWriter::with_capacity(20);
    w.write_varint(entity_id);
    w.write_varint(effect_id as i32);
    w.write_varint(i32::from(amplifier));
    w.write_varint(duration_ticks.max(0));
    w.write_u8(0x06); // FLAG_VISIBLE (0x02) | FLAG_SHOW_ICON (0x04)
    w.into_inner()
}

/// Encodes bytecode-verified MC `26.3` `ClientboundRemoveMobEffectPacket` (`s2c::REMOVE_MOB_EFFECT = 0x4F`):
/// `(entityId: VarInt, effect: holderRegistry VarInt 0-based)`.
pub fn encode_remove_mob_effect(entity_id: i32, effect_id: u32) -> Vec<u8> {
    let mut w = PacketWriter::with_capacity(10);
    w.write_varint(entity_id);
    w.write_varint(effect_id as i32);
    w.into_inner()
}

/// Encodes `ClientboundHurtAnimationPacket` (`id: VarInt, yaw: f32`).
pub fn encode_hurt_animation(entity_id: i32, hurt_yaw_deg: f32) -> Vec<u8> {
    let mut w = PacketWriter::with_capacity(10);
    w.write_varint(entity_id);
    w.write_f32(hurt_yaw_deg);
    w.into_inner()
}

/// Encodes `ClientboundAnimatePacket` (`id: VarInt, action: u8` -> `0 = SWING_MAIN_HAND`).
pub fn encode_animate(entity_id: i32, action: u8) -> Vec<u8> {
    let mut w = PacketWriter::with_capacity(8);
    w.write_varint(entity_id);
    w.write_u8(action);
    w.into_inner()
}

/// Encodes `ClientboundLevelEventPacket` (`s2c::LEVEL_EVENT = 0x2F`):
/// `(type: i32, pos: BlockPos, data: i32, globalEvent: bool)` — `type = 2001` spawns block-break particles + SFX!
pub fn encode_level_event(
    event_type: i32,
    pos: (i32, i32, i32),
    data: i32,
    global: bool,
) -> Vec<u8> {
    let mut w = PacketWriter::with_capacity(20);
    w.write_i32(event_type);
    w.write_block_pos(pos.0, pos.1, pos.2);
    w.write_i32(data);
    w.write_bool(global);
    w.into_inner()
}

/// Encodes `ClientboundBlockDestructionPacket` (`s2c::BLOCK_DESTRUCTION = 0x05`):
/// `(id: VarInt, pos: BlockPos, progress: u8)`.
pub fn encode_block_destruction(breaker_id: i32, pos: (i32, i32, i32), progress: u8) -> Vec<u8> {
    let mut w = PacketWriter::with_capacity(16);
    w.write_varint(breaker_id);
    w.write_block_pos(pos.0, pos.1, pos.2);
    w.write_u8(progress);
    w.into_inner()
}

/// Encodes MC `26.3` `ClientboundSetTimePacket` (`s2c::SET_TIME = 0x73`):
/// `(gameTime: i64, clockUpdates: Map<Holder<WorldClock>, ClockNetworkState>)`
/// where Overworld `WorldClock` holder ID is `0` (`holderRegistry`), followed by `(VarLong totalTicks, f32 partialTick, f32 rate)`.
pub fn encode_set_time_26_3(game_time: i64, day_time: i64, rate: f32) -> Vec<u8> {
    let mut w = PacketWriter::with_capacity(28);
    w.write_i64(game_time);
    w.write_varint(1); // 1 clock entry (minecraft:overworld)
    w.write_varint(0); // Registries.WORLD_CLOCK index 0 = minecraft:overworld
    w.write_varlong(day_time);
    w.write_f32(0.0); // partialTick
    w.write_f32(rate);
    w.into_inner()
}

/// Encodes MC `26.3` `ClientboundSetPlayerInventoryPacket` (`s2c::SET_PLAYER_INVENTORY = 0x6E`):
/// `(slot: VarInt, contents: ItemStack.OPTIONAL_STREAM_CODEC)`.
pub fn encode_set_player_inventory_slot(slot: usize, item_id: u32, count: u8) -> Vec<u8> {
    let mut w = PacketWriter::with_capacity(16);
    w.write_varint(slot as i32);
    if item_id == 0 || count == 0 {
        w.write_varint(0); // ItemStack.EMPTY
    } else {
        w.write_varint(i32::from(count));
        w.write_varint(item_id as i32); // holderRegistry(Registries.ITEM) 0-based ID
        w.write_varint(0); // DataComponentPatch added = 0
        w.write_varint(0); // DataComponentPatch removed = 0
    }
    w.into_inner()
}

/// Encodes `ClientboundTakeItemEntityPacket` (`s2c::TAKE_ITEM_ENTITY = 0x7F`):
/// `(itemId: VarInt, playerId: VarInt, amount: VarInt)`.
pub fn encode_take_item_entity(
    item_entity_id: i32,
    collector_entity_id: i32,
    amount: u8,
) -> Vec<u8> {
    let mut w = PacketWriter::with_capacity(15);
    w.write_varint(item_entity_id);
    w.write_varint(collector_entity_id);
    w.write_varint(i32::from(amount));
    w.into_inner()
}

/// Decodes `ServerboundUseItemPacket` (`c2s::USE_ITEM = 0x43`): `(hand: VarInt, sequence: VarInt, yRot: f32, xRot: f32)`.
pub fn decode_use_item_rot(r: &mut PacketReader<'_>) -> ProtocolResult<(i32, i32, f32, f32)> {
    let hand = r.read_varint()?;
    let seq = r.read_varint()?;
    let y_rot = r.read_f32()?;
    let x_rot = r.read_f32()?;
    Ok((hand, seq, y_rot, x_rot))
}

pub fn decode_use_item(r: &mut PacketReader<'_>) -> ProtocolResult<(i32, i32)> {
    let (hand, seq, _, _) = decode_use_item_rot(r)?;
    Ok((hand, seq))
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
    fn attack_time_inventory_and_command_suggestions_roundtrip() {
        let mut w = PacketWriter::new();
        w.write_varint(77);
        let mut r = PacketReader::new(w.as_slice());
        assert_eq!(decode_attack_entity(&mut r).expect("attack"), 77);

        let inv_pkt = encode_set_player_inventory_slot(3, 1050, 1);
        let mut ir = PacketReader::new(&inv_pkt);
        assert_eq!(ir.read_varint().unwrap(), 3);
        assert_eq!(ir.read_varint().unwrap(), 1);
        assert_eq!(ir.read_varint().unwrap(), 1050);

        let time_pkt = encode_set_time_26_3(1200, 6000, 1.0);
        let mut tr = PacketReader::new(&time_pkt);
        assert_eq!(tr.read_i64().unwrap(), 1200);
        assert_eq!(tr.read_varint().unwrap(), 1);
        assert_eq!(tr.read_varint().unwrap(), 0);
        assert_eq!(tr.read_varlong().unwrap(), 6000);

        let pkt = encode_command_suggestions(5, 1, 2, &["gamemode".into(), "give".into()]);
        let mut sr = PacketReader::new(&pkt);
        assert_eq!(sr.read_varint().unwrap(), 5);
        assert_eq!(sr.read_varint().unwrap(), 1);
        assert_eq!(sr.read_varint().unwrap(), 2);
        assert_eq!(sr.read_varint().unwrap(), 2);
        assert_eq!(sr.read_string(64).unwrap(), "gamemode");
    }
}
