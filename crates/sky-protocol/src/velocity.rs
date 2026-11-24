use crate::buf::PacketReader;
use crate::error::{ProtocolError, ProtocolResult};
use crate::packets::ProfileProperty;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

/// Channel identifier used by Velocity modern player info forwarding during Login.
pub const VELOCITY_PLAYER_INFO_CHANNEL: &str = "velocity:player_info";

/// Supported Velocity modern forwarding version requested by Solara-Sky (`MODERN_LAZY_SESSION` = 4).
pub const VELOCITY_MAX_SUPPORTED_VERSION: u8 = 4;

/// Verified player identity and real client address extracted from Velocity modern forwarding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VelocityForwardedPlayer {
    pub version: i32,
    pub client_address: String,
    pub uuid: Uuid,
    pub username: String,
    pub properties: Vec<ProfileProperty>,
}

/// Verifies the 32-byte HMAC-SHA256 prefix and decodes a `velocity:player_info` response payload.
pub fn verify_velocity_player_info(
    payload: &[u8],
    forwarding_secret: &[u8],
) -> ProtocolResult<VelocityForwardedPlayer> {
    if payload.len() < 32 {
        return Err(ProtocolError::UnexpectedEof {
            needed: 32,
            available: payload.len(),
        });
    }
    let (signature, data) = payload.split_at(32);

    let mut mac =
        HmacSha256::new_from_slice(forwarding_secret).expect("HMAC-SHA256 accepts any key size");
    mac.update(data);
    mac.verify_slice(signature)
        .map_err(|_| ProtocolError::InvalidVelocitySignature)?;

    let mut r = PacketReader::new(data);
    let version = r.read_varint()?;
    if !(1..=4).contains(&version) {
        return Err(ProtocolError::UnsupportedVelocityVersion(version));
    }

    let client_address = r.read_string(256)?;
    let uuid = r.read_uuid()?;
    let username = r.read_string(16)?;

    let prop_count = r.read_usize_bounded(64)?;
    let mut properties = Vec::with_capacity(prop_count);
    for _ in 0..prop_count {
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

    Ok(VelocityForwardedPlayer {
        version,
        client_address,
        uuid,
        username,
        properties,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buf::PacketWriter;

    #[test]
    fn verifies_valid_and_rejects_tampered_velocity_forwarding() {
        let secret = b"super-secret-velocity-forwarding-key";
        let expected_uuid = Uuid::new_v4();

        let mut body = PacketWriter::new();
        body.write_varint(4); // MODERN_LAZY_SESSION
        body.write_string("203.0.113.42");
        body.write_uuid(expected_uuid);
        body.write_string("SolaraPlayer");
        body.write_varint(1);
        body.write_string("textures");
        body.write_string("base64-skin-data");
        body.write_bool(true);
        body.write_string("mojang-sig");

        let mut mac = HmacSha256::new_from_slice(secret).expect("hmac");
        mac.update(body.as_slice());
        let sig = mac.finalize().into_bytes();

        let mut full = Vec::new();
        full.extend_from_slice(&sig);
        full.extend_from_slice(body.as_slice());

        let parsed = verify_velocity_player_info(&full, secret).expect("valid velocity payload");
        assert_eq!(parsed.version, 4);
        assert_eq!(parsed.client_address, "203.0.113.42");
        assert_eq!(parsed.uuid, expected_uuid);
        assert_eq!(parsed.username, "SolaraPlayer");
        assert_eq!(parsed.properties.len(), 1);

        // Tamper with username byte -> must fail HMAC verification
        let mut tampered = full.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 0xFF;
        assert!(matches!(
            verify_velocity_player_info(&tampered, secret),
            Err(ProtocolError::InvalidVelocitySignature)
        ));
    }
}
