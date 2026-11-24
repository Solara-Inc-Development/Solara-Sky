use crate::buf::{PacketReader, PacketWriter};
use crate::error::{ProtocolError, ProtocolResult};
use crate::{MINECRAFT_VERSION, PROTOCOL_VERSION, generated_packets};
use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Connection state intent sent in the initial Handshake packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientIntent {
    Status = 1,
    Login = 2,
    Transfer = 3,
}

impl ClientIntent {
    pub fn from_id(id: i32) -> ProtocolResult<Self> {
        match id {
            1 => Ok(Self::Status),
            2 => Ok(Self::Login),
            3 => Ok(Self::Transfer),
            other => Err(ProtocolError::InvalidIntent(other)),
        }
    }
}

/// Serverbound Handshake (`minecraft:intention`, state = Handshake, ID = 0x00).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandshakeC2s {
    pub protocol_version: i32,
    pub server_address: String,
    pub server_port: u16,
    pub intent: ClientIntent,
}

impl HandshakeC2s {
    pub const PACKET_ID: i32 = generated_packets::handshake::serverbound::INTENTION;

    pub fn decode(r: &mut PacketReader<'_>) -> ProtocolResult<Self> {
        // Allow up to 32767 chars when BungeeCord forwarding injects skin properties into hostName
        let protocol_version = r.read_varint()?;
        let server_address = r.read_string(32767)?;
        let server_port = r.read_u16()?;
        let intent = ClientIntent::from_id(r.read_varint()?)?;
        Ok(Self {
            protocol_version,
            server_address,
            server_port,
            intent,
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = PacketWriter::new();
        w.write_varint(self.protocol_version);
        w.write_string(&self.server_address);
        w.write_u16(self.server_port);
        w.write_varint(self.intent as i32);
        w.into_inner()
    }
}

/// Game profile property (e.g., `textures` skin/cape data).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileProperty {
    pub name: String,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

/// Parsed BungeeCord legacy IP-forwarding data embedded in `HandshakeC2s::server_address`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BungeeForwardedClient {
    pub server_host: String,
    pub client_ip: String,
    pub uuid: Uuid,
    pub properties: Vec<ProfileProperty>,
}

pub fn parse_bungeecord_address(raw_host: &str) -> ProtocolResult<BungeeForwardedClient> {
    let mut parts = raw_host.split('\0');
    let server_host = parts
        .next()
        .ok_or(ProtocolError::InvalidBungeePayload)?
        .to_owned();
    let client_ip = parts
        .next()
        .ok_or(ProtocolError::InvalidBungeePayload)?
        .to_owned();
    let uuid_str = parts.next().ok_or(ProtocolError::InvalidBungeePayload)?;
    let uuid = Uuid::parse_str(uuid_str).map_err(|_| ProtocolError::InvalidBungeePayload)?;
    let properties = match parts.next() {
        Some(json_str) if !json_str.is_empty() => serde_json::from_str(json_str)?,
        _ => Vec::new(),
    };
    Ok(BungeeForwardedClient {
        server_host,
        client_ip,
        uuid,
        properties,
    })
}

/// Computes the standard Minecraft offline-mode UUIDv3 (`OfflinePlayer:<username>`).
pub fn offline_player_uuid(username: &str) -> Uuid {
    let mut md5 = Md5::new();
    md5.update(b"OfflinePlayer:");
    md5.update(username.as_bytes());
    let mut bytes: [u8; 16] = md5.finalize().into();
    bytes[6] = (bytes[6] & 0x0f) | 0x30; // Version 3 (MD5)
    bytes[8] = (bytes[8] & 0x3f) | 0x80; // RFC 4122 variant
    Uuid::from_bytes(bytes)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatusVersion {
    pub name: String,
    pub protocol: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatusPlayerSample {
    pub name: String,
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatusPlayers {
    pub max: usize,
    pub online: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sample: Vec<StatusPlayerSample>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatusTextComponent {
    pub text: String,
}

/// JSON payload returned in `minecraft:status_response` (Server List Ping).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerStatusJson {
    pub version: StatusVersion,
    pub players: StatusPlayers,
    pub description: StatusTextComponent,
    #[serde(rename = "enforcesSecureChat")]
    pub enforces_secure_chat: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub favicon: Option<String>,
}

impl ServerStatusJson {
    pub fn new(motd: impl Into<String>, online: usize, max: usize) -> Self {
        Self {
            version: StatusVersion {
                name: MINECRAFT_VERSION.to_owned(),
                protocol: PROTOCOL_VERSION,
            },
            players: StatusPlayers {
                max,
                online,
                sample: Vec::new(),
            },
            description: StatusTextComponent { text: motd.into() },
            enforces_secure_chat: false,
            favicon: None,
        }
    }

    pub fn encode_packet(&self) -> ProtocolResult<Vec<u8>> {
        let json = serde_json::to_string(self)?;
        let mut w = PacketWriter::new();
        w.write_string(&json);
        Ok(w.into_inner())
    }
}

/// Serverbound Login Start (`minecraft:hello`, ID = 0x00).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginHelloC2s {
    pub username: String,
    pub profile_id: Uuid,
}

impl LoginHelloC2s {
    pub const PACKET_ID: i32 = generated_packets::login::serverbound::HELLO;

    pub fn decode(r: &mut PacketReader<'_>) -> ProtocolResult<Self> {
        Ok(Self {
            username: r.read_string(16)?,
            profile_id: r.read_uuid()?,
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = PacketWriter::new();
        w.write_string(&self.username);
        w.write_uuid(self.profile_id);
        w.into_inner()
    }
}

/// Clientbound Encryption Request (`minecraft:hello`, ID = 0x01 in 26.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginHelloS2c {
    pub server_id: String,
    pub public_key_der: Vec<u8>,
    pub verify_token: Vec<u8>,
    pub should_authenticate: bool,
}

impl LoginHelloS2c {
    pub const PACKET_ID: i32 = generated_packets::login::clientbound::HELLO;

    pub fn encode(&self) -> Vec<u8> {
        let mut w = PacketWriter::new();
        w.write_string(&self.server_id);
        w.write_byte_array(&self.public_key_der);
        w.write_byte_array(&self.verify_token);
        w.write_bool(self.should_authenticate);
        w.into_inner()
    }

    pub fn decode(r: &mut PacketReader<'_>) -> ProtocolResult<Self> {
        Ok(Self {
            server_id: r.read_string(20)?,
            public_key_der: r.read_byte_array(2048)?,
            verify_token: r.read_byte_array(512)?,
            should_authenticate: r.read_bool()?,
        })
    }
}

/// Serverbound Encryption Response (`minecraft:key`, ID = 0x01 in 26.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginKeyC2s {
    pub shared_secret_encrypted: Vec<u8>,
    pub verify_token_encrypted: Vec<u8>,
}

impl LoginKeyC2s {
    pub const PACKET_ID: i32 = generated_packets::login::serverbound::KEY;

    pub fn decode(r: &mut PacketReader<'_>) -> ProtocolResult<Self> {
        Ok(Self {
            shared_secret_encrypted: r.read_byte_array(1024)?,
            verify_token_encrypted: r.read_byte_array(1024)?,
        })
    }
}

/// Clientbound Set Compression (`minecraft:login_compression`, ID = 0x03 in 26.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoginCompressionS2c {
    pub threshold: i32,
}

impl LoginCompressionS2c {
    pub const PACKET_ID: i32 = generated_packets::login::clientbound::LOGIN_COMPRESSION;

    pub fn encode(&self) -> Vec<u8> {
        let mut w = PacketWriter::new();
        w.write_varint(self.threshold);
        w.into_inner()
    }
}

/// Clientbound Login Finished (`minecraft:login_finished`, ID = 0x02 in 26.3).
///
/// Verified against `ClientboundLoginFinishedPacket.class` in MC `26.3`:
/// `GameProfile(uuid, name, properties)` followed by `sessionId: UUID`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginFinishedS2c {
    pub uuid: Uuid,
    pub username: String,
    pub properties: Vec<ProfileProperty>,
    pub session_id: Uuid,
}

impl LoginFinishedS2c {
    pub const PACKET_ID: i32 = generated_packets::login::clientbound::LOGIN_FINISHED;

    pub fn encode(&self) -> Vec<u8> {
        let mut w = PacketWriter::new();
        w.write_uuid(self.uuid);
        w.write_string(&self.username);
        w.write_varint(self.properties.len() as i32);
        for prop in &self.properties {
            w.write_string(&prop.name);
            w.write_string(&prop.value);
            match &prop.signature {
                Some(sig) => {
                    w.write_bool(true);
                    w.write_string(sig);
                }
                None => w.write_bool(false),
            }
        }
        w.write_uuid(self.session_id);
        w.into_inner()
    }

    pub fn decode(r: &mut PacketReader<'_>) -> ProtocolResult<Self> {
        let uuid = r.read_uuid()?;
        let username = r.read_string(16)?;
        let count = r.read_usize_bounded(64)?;
        let mut properties = Vec::with_capacity(count);
        for _ in 0..count {
            let name = r.read_string(32767)?;
            let value = r.read_string(32767)?;
            let signature = if r.read_bool()? {
                Some(r.read_string(32767)?)
            } else {
                None
            };
            properties.push(ProfileProperty {
                name,
                value,
                signature,
            });
        }
        let session_id = r.read_uuid()?;
        Ok(Self {
            uuid,
            username,
            properties,
            session_id,
        })
    }
}

/// Clientbound Login Disconnect (`minecraft:login_disconnect`, ID = 0x00 in 26.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginDisconnectS2c {
    pub reason_text: String,
}

impl LoginDisconnectS2c {
    pub const PACKET_ID: i32 = generated_packets::login::clientbound::LOGIN_DISCONNECT;

    pub fn encode(&self) -> ProtocolResult<Vec<u8>> {
        let json = serde_json::to_string(&StatusTextComponent {
            text: self.reason_text.clone(),
        })?;
        let mut w = PacketWriter::new();
        w.write_string(&json);
        Ok(w.into_inner())
    }
}

/// Clientbound Login Custom Query (`minecraft:custom_query`, ID = 0x04 in 26.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginCustomQueryS2c {
    pub transaction_id: i32,
    pub channel: String,
    pub data: Vec<u8>,
}

impl LoginCustomQueryS2c {
    pub const PACKET_ID: i32 = generated_packets::login::clientbound::CUSTOM_QUERY;

    pub fn encode(&self) -> Vec<u8> {
        let mut w = PacketWriter::new();
        w.write_varint(self.transaction_id);
        w.write_string(&self.channel);
        w.write_slice(&self.data);
        w.into_inner()
    }
}

/// Serverbound Login Custom Query Answer (`minecraft:custom_query_answer`, ID = 0x02 in 26.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginCustomQueryAnswerC2s {
    pub transaction_id: i32,
    pub data: Option<Vec<u8>>,
}

impl LoginCustomQueryAnswerC2s {
    pub const PACKET_ID: i32 = generated_packets::login::serverbound::CUSTOM_QUERY_ANSWER;

    pub fn decode(r: &mut PacketReader<'_>) -> ProtocolResult<Self> {
        let transaction_id = r.read_varint()?;
        let data = if r.read_bool()? {
            if r.remaining() > 1_048_576 {
                return Err(ProtocolError::LengthTooLarge {
                    len: r.remaining(),
                    max: 1_048_576,
                });
            }
            Some(r.read_remaining().to_vec())
        } else {
            None
        };
        Ok(Self {
            transaction_id,
            data,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_uuid_matches_vanilla_java_algorithm() {
        // Known Vanilla Java Edition offline UUID for "Notch":
        // UUID.nameUUIDFromBytes("OfflinePlayer:Notch".getBytes(UTF_8))
        assert_eq!(
            offline_player_uuid("Notch").to_string(),
            "b50ad385-829d-3141-a216-7e7d7539ba7f"
        );
    }

    #[test]
    fn login_finished_26_3_roundtrip_includes_session_id() {
        let pkt = LoginFinishedS2c {
            uuid: Uuid::new_v4(),
            username: "SolaraDev".into(),
            properties: vec![ProfileProperty {
                name: "textures".into(),
                value: "abc".into(),
                signature: Some("sig".into()),
            }],
            session_id: Uuid::new_v4(),
        };
        let bytes = pkt.encode();
        let mut r = PacketReader::new(&bytes);
        let decoded = LoginFinishedS2c::decode(&mut r).expect("decode LoginFinishedS2c");
        assert_eq!(decoded, pkt);
        r.ensure_empty().expect("empty");
    }

    #[test]
    fn parses_bungeecord_handshake_address() {
        let uid = Uuid::new_v4();
        let addr = format!(
            "play.solara.gg\0198.51.100.7\0{}\0[{{\"name\":\"textures\",\"value\":\"xyz\"}}]",
            uid.simple()
        );
        let parsed = parse_bungeecord_address(&addr).expect("parse bungee");
        assert_eq!(parsed.server_host, "play.solara.gg");
        assert_eq!(parsed.client_ip, "198.51.100.7");
        assert_eq!(parsed.uuid, uid);
        assert_eq!(parsed.properties.len(), 1);
    }
}
