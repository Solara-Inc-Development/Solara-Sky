use crate::config::{ProxyMode, ServerProperties, SkyConfig};
use anyhow::{Context, Result, bail};
use rand::RngCore;
use rsa::pkcs8::EncodePublicKey;
use rsa::{Pkcs1v15Encrypt, RsaPrivateKey};
use serde::Deserialize;
use sky_api::{PlayerPreLoginEvent, PluginManager, StatusPingEvent};
use sky_protocol::buf::{PacketReader, PacketWriter};
use sky_protocol::crypto::{MinecraftCipher, mojang_server_hash};
use sky_protocol::error::ProtocolError;
use sky_protocol::frame::{MAX_WIRE_FRAME_BYTES, decode_frame_body, encode_frame};
use sky_protocol::generated_packets;
use sky_protocol::packets::{
    ClientIntent, HandshakeC2s, LoginCompressionS2c, LoginCustomQueryAnswerC2s,
    LoginCustomQueryS2c, LoginDisconnectS2c, LoginHelloC2s, LoginHelloS2c, LoginKeyC2s,
    ProfileProperty, ServerStatusJson, offline_player_uuid, parse_bungeecord_address,
};
use sky_protocol::velocity::{
    VELOCITY_MAX_SUPPORTED_VERSION, VELOCITY_PLAYER_INFO_CHANNEL, verify_velocity_player_info,
};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tracing::{debug, info, warn};
use uuid::Uuid;

pub struct SharedServerState {
    pub props: ServerProperties,
    pub sky_cfg: SkyConfig,
    pub rsa_key: Option<(RsaPrivateKey, Vec<u8>)>,
    pub http: reqwest::Client,
    pub plugins: PluginManager,
    pub online_players: AtomicUsize,
}

impl SharedServerState {
    pub fn new(
        props: ServerProperties,
        sky_cfg: SkyConfig,
        plugins: PluginManager,
    ) -> Result<Self> {
        let rsa_key = if props.online_mode && sky_cfg.proxy.mode == ProxyMode::None {
            let mut rng = rand::thread_rng();
            let priv_key =
                RsaPrivateKey::new(&mut rng, 1024).context("generate 1024-bit RSA keypair")?;
            let pub_der = priv_key
                .to_public_key()
                .to_public_key_der()
                .context("encode SPKI DER public key")?
                .into_vec();
            Some((priv_key, pub_der))
        } else {
            None
        };

        let http = reqwest::Client::builder()
            .user_agent("Solara-Sky/0.1")
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .context("build HTTP client")?;

        Ok(Self {
            props,
            sky_cfg,
            rsa_key,
            http,
            plugins,
            online_players: AtomicUsize::new(0),
        })
    }
}

struct FramedConnection {
    stream: TcpStream,
    cipher: Option<MinecraftCipher>,
    compression_threshold: Option<usize>,
}

impl FramedConnection {
    fn new(stream: TcpStream) -> Self {
        Self {
            stream,
            cipher: None,
            compression_threshold: None,
        }
    }

    fn enable_encryption(&mut self, secret: &[u8; 16]) {
        self.cipher = Some(MinecraftCipher::new(secret));
    }

    fn set_compression(&mut self, threshold: i32) {
        self.compression_threshold = (threshold >= 0).then_some(threshold as usize);
    }

    async fn read_byte(&mut self) -> Result<Option<u8>, ProtocolError> {
        let mut b = [0u8; 1];
        match self.stream.read_exact(&mut b).await {
            Ok(_) => {
                if let Some(c) = &mut self.cipher {
                    c.decrypt_in_place(&mut b);
                }
                Ok(Some(b[0]))
            }
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(None),
            Err(e) => Err(ProtocolError::Io(e)),
        }
    }

    async fn read_packet(&mut self) -> Result<Option<(i32, Vec<u8>)>, ProtocolError> {
        // Read frame length VarInt byte-by-byte (graceful None if client closed cleanly at boundary)
        let mut num_read = 0usize;
        let mut frame_len_u32 = 0u32;
        loop {
            let maybe = self.read_byte().await?;
            let Some(byte) = maybe else {
                if num_read == 0 {
                    return Ok(None);
                }
                return Err(ProtocolError::UnexpectedEof {
                    needed: 1,
                    available: 0,
                });
            };
            if num_read >= 5 {
                return Err(ProtocolError::VarIntTooLarge);
            }
            frame_len_u32 |= u32::from(byte & 0x7F) << (7 * num_read);
            num_read += 1;
            if (byte & 0x80) == 0 {
                break;
            }
        }

        let frame_len_i32 = frame_len_u32 as i32;
        if frame_len_i32 < 0 {
            return Err(ProtocolError::NegativeLength(frame_len_i32));
        }
        let frame_len = frame_len_i32 as usize;
        if frame_len > MAX_WIRE_FRAME_BYTES {
            return Err(ProtocolError::PacketTooLarge {
                len: frame_len,
                max: MAX_WIRE_FRAME_BYTES,
            });
        }

        let mut body = vec![0u8; frame_len];
        self.stream.read_exact(&mut body).await?;
        if let Some(c) = &mut self.cipher {
            c.decrypt_in_place(&mut body);
        }
        let decoded = decode_frame_body(&body, self.compression_threshold.is_some())?;
        Ok(Some(decoded))
    }

    async fn send_packet(&mut self, packet_id: i32, payload: &[u8]) -> Result<(), ProtocolError> {
        let mut wire = encode_frame(packet_id, payload, self.compression_threshold)?;
        if let Some(c) = &mut self.cipher {
            c.encrypt_in_place(&mut wire);
        }
        self.stream.write_all(&wire).await?;
        self.stream.flush().await?;
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct MojangHasJoinedResponse {
    id: String,
    name: String,
    #[serde(default)]
    properties: Vec<ProfileProperty>,
}

pub async fn handle_client(
    stream: TcpStream,
    peer: SocketAddr,
    state: Arc<SharedServerState>,
) -> Result<()> {
    let _ = stream.set_nodelay(true);
    let mut conn = FramedConnection::new(stream);

    let Some((pkt_id, payload)) = conn.read_packet().await? else {
        return Ok(());
    };
    if pkt_id != HandshakeC2s::PACKET_ID {
        bail!("expected Handshake (0x00) from {peer}, got 0x{pkt_id:02X}");
    }
    let mut r = PacketReader::new(&payload);
    let handshake = HandshakeC2s::decode(&mut r)?;
    debug!(
        %peer,
        protocol = handshake.protocol_version,
        intent = ?handshake.intent,
        "received handshake"
    );

    match handshake.intent {
        ClientIntent::Status => handle_status(&mut conn, &state).await,
        ClientIntent::Login | ClientIntent::Transfer => {
            handle_login(&mut conn, peer, &handshake, &state).await
        }
    }
}

async fn handle_status(conn: &mut FramedConnection, state: &SharedServerState) -> Result<()> {
    while let Some((pkt_id, payload)) = conn.read_packet().await? {
        match pkt_id {
            generated_packets::status::serverbound::STATUS_REQUEST => {
                let mut event = StatusPingEvent {
                    description: state.props.motd.clone(),
                    online_players: state.online_players.load(Ordering::Relaxed),
                    max_players: state.props.max_players,
                    protocol_version: sky_protocol::PROTOCOL_VERSION,
                };
                state.plugins.fire_status_ping(&mut event);

                let mut status = ServerStatusJson::new(
                    event.description,
                    event.online_players,
                    event.max_players,
                );
                status.version.protocol = event.protocol_version;
                let encoded = status.encode_packet()?;
                conn.send_packet(
                    generated_packets::status::clientbound::STATUS_RESPONSE,
                    &encoded,
                )
                .await?;
            }
            generated_packets::status::serverbound::PING_REQUEST => {
                let mut r = PacketReader::new(&payload);
                let client_time = r.read_i64()?;
                let mut w = PacketWriter::with_capacity(8);
                w.write_i64(client_time);
                conn.send_packet(
                    generated_packets::status::clientbound::PONG_RESPONSE,
                    w.as_slice(),
                )
                .await?;
                return Ok(());
            }
            other => bail!("unexpected Status packet 0x{other:02X}"),
        }
    }
    Ok(())
}

async fn send_login_disconnect(conn: &mut FramedConnection, msg: impl Into<String>) -> Result<()> {
    let pkt = LoginDisconnectS2c {
        reason_text: msg.into(),
    };
    let bytes = pkt.encode()?;
    conn.send_packet(LoginDisconnectS2c::PACKET_ID, &bytes)
        .await?;
    Ok(())
}

async fn handle_login(
    conn: &mut FramedConnection,
    peer: SocketAddr,
    handshake: &HandshakeC2s,
    state: &SharedServerState,
) -> Result<()> {
    if handshake.protocol_version != sky_protocol::PROTOCOL_VERSION {
        send_login_disconnect(
            conn,
            format!(
                "Incompatible Minecraft version (Solara-Sky expects {} / protocol {})",
                sky_protocol::MINECRAFT_VERSION,
                sky_protocol::PROTOCOL_VERSION
            ),
        )
        .await?;
        return Ok(());
    }

    let Some((pkt_id, payload)) = conn.read_packet().await? else {
        return Ok(());
    };
    if pkt_id != LoginHelloC2s::PACKET_ID {
        bail!("expected LoginHello (0x00) from {peer}, got 0x{pkt_id:02X}");
    }
    let mut r = PacketReader::new(&payload);
    let hello = LoginHelloC2s::decode(&mut r)?;

    let (uuid, username, properties, effective_ip) = match state.sky_cfg.proxy.mode {
        ProxyMode::Velocity => {
            if state.sky_cfg.proxy.velocity_secret.is_empty() {
                send_login_disconnect(conn, "Velocity forwarding secret is not configured.")
                    .await?;
                return Ok(());
            }
            let tx_id = 1042;
            let query = LoginCustomQueryS2c {
                transaction_id: tx_id,
                channel: VELOCITY_PLAYER_INFO_CHANNEL.into(),
                data: vec![VELOCITY_MAX_SUPPORTED_VERSION],
            };
            conn.send_packet(LoginCustomQueryS2c::PACKET_ID, &query.encode())
                .await?;

            let Some((ans_id, ans_payload)) = conn.read_packet().await? else {
                return Ok(());
            };
            if ans_id != LoginCustomQueryAnswerC2s::PACKET_ID {
                bail!("expected CustomQueryAnswer for Velocity forwarding");
            }
            let mut ar = PacketReader::new(&ans_payload);
            let answer = LoginCustomQueryAnswerC2s::decode(&mut ar)?;
            if answer.transaction_id != tx_id {
                send_login_disconnect(conn, "Mismatched Velocity query transaction ID.").await?;
                return Ok(());
            }
            let Some(raw_info) = answer.data else {
                send_login_disconnect(
                    conn,
                    "Direct connections are disabled — please connect through Velocity.",
                )
                .await?;
                return Ok(());
            };
            match verify_velocity_player_info(
                &raw_info,
                state.sky_cfg.proxy.velocity_secret.as_bytes(),
            ) {
                Ok(vp) => (vp.uuid, vp.username, vp.properties, vp.client_address),
                Err(err) => {
                    warn!(%peer, ?err, "Velocity forwarding verification failed");
                    send_login_disconnect(conn, "Invalid Velocity forwarding signature.").await?;
                    return Ok(());
                }
            }
        }
        ProxyMode::Bungeecord => match parse_bungeecord_address(&handshake.server_address) {
            Ok(bc) => (bc.uuid, hello.username, bc.properties, bc.client_ip),
            Err(_) => {
                send_login_disconnect(
                    conn,
                    "If you wish to use IP forwarding, please enable it in your BungeeCord config as well!",
                )
                .await?;
                return Ok(());
            }
        },
        ProxyMode::None => {
            if state.props.online_mode {
                let Some((priv_key, pub_der)) = &state.rsa_key else {
                    bail!("online-mode active without RSA keypair");
                };
                let mut challenge = [0u8; 4];
                rand::thread_rng().fill_bytes(&mut challenge);

                let enc_req = LoginHelloS2c {
                    server_id: String::new(),
                    public_key_der: pub_der.clone(),
                    verify_token: challenge.to_vec(),
                    should_authenticate: true,
                };
                conn.send_packet(LoginHelloS2c::PACKET_ID, &enc_req.encode())
                    .await?;

                let Some((key_id, key_payload)) = conn.read_packet().await? else {
                    return Ok(());
                };
                if key_id != LoginKeyC2s::PACKET_ID {
                    bail!("expected LoginKeyC2s (0x01), got 0x{key_id:02X}");
                }
                let mut kr = PacketReader::new(&key_payload);
                let key_resp = LoginKeyC2s::decode(&mut kr)?;

                let dec_token = priv_key
                    .decrypt(Pkcs1v15Encrypt, &key_resp.verify_token_encrypted)
                    .context("RSA decrypt verify token")?;
                if dec_token.as_slice() != challenge {
                    send_login_disconnect(conn, "Encryption verify token mismatch.").await?;
                    return Ok(());
                }

                let dec_secret = priv_key
                    .decrypt(Pkcs1v15Encrypt, &key_resp.shared_secret_encrypted)
                    .context("RSA decrypt shared secret")?;
                let Ok(secret_arr): Result<[u8; 16], _> = dec_secret.try_into() else {
                    send_login_disconnect(conn, "Invalid shared secret length.").await?;
                    return Ok(());
                };

                // Enable AES-128-CFB8 stream encryption immediately after ServerboundKeyPacket
                conn.enable_encryption(&secret_arr);

                let server_hash = mojang_server_hash("", &secret_arr, pub_der);
                let resp = state
                    .http
                    .get("https://sessionserver.mojang.com/session/minecraft/hasJoined")
                    .query(&[
                        ("username", hello.username.as_str()),
                        ("serverId", server_hash.as_str()),
                    ])
                    .send()
                    .await
                    .context("query Mojang sessionserver")?;

                if resp.status() != reqwest::StatusCode::OK {
                    send_login_disconnect(
                        conn,
                        "Failed to verify username with Mojang session servers!",
                    )
                    .await?;
                    return Ok(());
                }

                let joined: MojangHasJoinedResponse =
                    resp.json().await.context("parse Mojang hasJoined JSON")?;
                let verified_uuid =
                    Uuid::parse_str(&joined.id).context("parse Mojang player UUID")?;
                (
                    verified_uuid,
                    joined.name,
                    joined.properties,
                    peer.ip().to_string(),
                )
            } else {
                let uid = offline_player_uuid(&hello.username);
                (uid, hello.username, Vec::new(), peer.ip().to_string())
            }
        }
    };

    if state.props.network_compression_threshold >= 0 {
        let comp = LoginCompressionS2c {
            threshold: state.props.network_compression_threshold,
        };
        conn.send_packet(LoginCompressionS2c::PACKET_ID, &comp.encode())
            .await?;
        conn.set_compression(state.props.network_compression_threshold);
    }

    let mut pre_login = PlayerPreLoginEvent {
        username: username.clone(),
        uuid,
        cancel_reason: None,
    };
    state.plugins.fire_pre_login(&mut pre_login);
    if let Some(reason) = pre_login.cancel_reason {
        send_login_disconnect(conn, reason).await?;
        return Ok(());
    }

    info!(
        username = %username,
        %uuid,
        ip = %effective_ip,
        props = properties.len(),
        "authenticated player login"
    );

    // Phase 0/1 honest login completion notice until Phase 2 Configuration + Play world streaming lands.
    send_login_disconnect(
        conn,
        format!(
            "Solara-Sky MC 26.3: Authenticated {} ({})! World streaming lands in Phase 2.",
            username, uuid
        ),
    )
    .await?;
    Ok(())
}
