use crate::config::{ProxyMode, ServerProperties, SkyConfig};
use anyhow::{Context, Result, bail};
use rand::RngCore;
use rsa::pkcs8::EncodePublicKey;
use rsa::{Pkcs1v15Encrypt, RsaPrivateKey};
use serde::Deserialize;
use sky_api::{PlayerPreLoginEvent, PluginManager, StatusPingEvent};
use sky_protocol::buf::{PacketReader, PacketWriter};
use sky_protocol::config_play::{
    AcceptTeleportC2s, GAME_EVENT_START_WAITING_CHUNKS, GameMode, KnownPack, PlayLoginS2c,
    decode_select_known_packs, encode_brand_payload, encode_enabled_features, encode_game_event,
    encode_nbt_disconnect, encode_player_position, encode_registry_data_known,
    encode_select_known_packs, encode_set_chunk_cache_center, encode_update_tags,
};
use sky_protocol::crypto::{MinecraftCipher, mojang_server_hash};
use sky_protocol::error::ProtocolError;
use sky_protocol::frame::{MAX_WIRE_FRAME_BYTES, decode_frame_body, encode_frame};
use sky_protocol::packets::{
    ClientIntent, HandshakeC2s, LoginCompressionS2c, LoginCustomQueryAnswerC2s,
    LoginCustomQueryS2c, LoginDisconnectS2c, LoginFinishedS2c, LoginHelloC2s, LoginHelloS2c,
    LoginKeyC2s, ProfileProperty, ServerStatusJson, offline_player_uuid, parse_bungeecord_address,
};
use sky_protocol::velocity::{
    VELOCITY_MAX_SUPPORTED_VERSION, VELOCITY_PLAYER_INFO_CHANNEL, verify_velocity_player_info,
};
use sky_protocol::{generated_packets, generated_registries};
use sky_world::{ChunkPos, WorldChunk};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
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
    pub next_entity_id: AtomicI32,
    /// Pre-serialized `ClientboundLevelChunkWithLightPacket` payloads for the spawn region.
    pub spawn_chunk_packets: Vec<Vec<u8>>,
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
            .timeout(Duration::from_secs(10))
            .build()
            .context("build HTTP client")?;

        let radius = i32::from(props.view_distance.clamp(2, 6));
        let mut spawn_chunk_packets =
            Vec::with_capacity(((2 * radius + 1) * (2 * radius + 1)) as usize);
        for cz in -radius..=radius {
            for cx in -radius..=radius {
                let chunk = WorldChunk::generate_spawn_world(ChunkPos { x: cx, z: cz });
                spawn_chunk_packets.push(chunk.encode_level_chunk_with_light(false));
            }
        }

        Ok(Self {
            props,
            sky_cfg,
            rsa_key,
            http,
            plugins,
            online_players: AtomicUsize::new(0),
            next_entity_id: AtomicI32::new(1),
            spawn_chunk_packets,
        })
    }
}

struct OnlinePlayerGuard<'a>(&'a AtomicUsize);

impl<'a> OnlinePlayerGuard<'a> {
    fn acquire(counter: &'a AtomicUsize) -> Self {
        counter.fetch_add(1, Ordering::Relaxed);
        Self(counter)
    }
}

impl Drop for OnlinePlayerGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
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

    if state.online_players.load(Ordering::Relaxed) >= state.props.max_players {
        send_login_disconnect(conn, "The server is full!").await?;
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

    let session_id = Uuid::new_v4();
    let finished = LoginFinishedS2c {
        uuid,
        username: username.clone(),
        properties,
        session_id,
    };
    conn.send_packet(LoginFinishedS2c::PACKET_ID, &finished.encode())
        .await?;

    // Wait for ServerboundLoginAcknowledgedPacket to enter Configuration state
    let Some((ack_id, _)) = conn.read_packet().await? else {
        return Ok(());
    };
    if ack_id != generated_packets::login::serverbound::LOGIN_ACKNOWLEDGED {
        bail!("expected LoginAcknowledged (0x03), got 0x{ack_id:02X}");
    }

    info!(username = %username, %uuid, ip = %effective_ip, "player entered Configuration state");
    if !handle_configuration(conn, &username).await? {
        return Ok(());
    }

    handle_play(conn, &username, uuid, state).await
}

async fn handle_configuration(conn: &mut FramedConnection, username: &str) -> Result<bool> {
    use generated_packets::configuration::{clientbound as s2c, serverbound as c2s};

    conn.send_packet(s2c::CUSTOM_PAYLOAD, &encode_brand_payload("solara-sky"))
        .await?;
    conn.send_packet(
        s2c::UPDATE_ENABLED_FEATURES,
        &encode_enabled_features(&["minecraft:vanilla"]),
    )
    .await?;
    let core_pack = KnownPack::vanilla_core();
    conn.send_packet(
        s2c::SELECT_KNOWN_PACKS,
        &encode_select_known_packs(std::slice::from_ref(&core_pack)),
    )
    .await?;

    while let Some((pkt_id, payload)) = conn.read_packet().await? {
        match pkt_id {
            c2s::CLIENT_INFORMATION | c2s::CUSTOM_PAYLOAD | c2s::RESOURCE_PACK => {
                // Accepted cleanly
            }
            c2s::KEEP_ALIVE | c2s::PONG => {}
            c2s::SELECT_KNOWN_PACKS => {
                let mut r = PacketReader::new(&payload);
                let client_packs = decode_select_known_packs(&mut r)?;
                if !client_packs.contains(&core_pack) {
                    conn.send_packet(
                        s2c::DISCONNECT,
                        &encode_nbt_disconnect(
                            "Client did not advertise vanilla minecraft:core 26.3 data pack.",
                        ),
                    )
                    .await?;
                    return Ok(false);
                }

                // Stream all 32 synchronized dynamic registries + 773 vanilla network tags
                for &(reg_id, entries) in generated_registries::SYNCHRONIZED_REGISTRIES {
                    let reg_bytes = encode_registry_data_known(reg_id, entries);
                    conn.send_packet(s2c::REGISTRY_DATA, &reg_bytes).await?;
                }
                let tags_bytes = encode_update_tags(generated_registries::VANILLA_NETWORK_TAGS);
                conn.send_packet(s2c::UPDATE_TAGS, &tags_bytes).await?;

                conn.send_packet(s2c::FINISH_CONFIGURATION, &[]).await?;
            }
            c2s::FINISH_CONFIGURATION => {
                debug!(%username, "Configuration state completed");
                return Ok(true);
            }
            other => {
                debug!(%username, pkt_id = other, "ignoring unhandled Configuration packet");
            }
        }
    }
    Ok(false)
}

async fn handle_play(
    conn: &mut FramedConnection,
    username: &str,
    uuid: Uuid,
    state: &SharedServerState,
) -> Result<()> {
    use generated_packets::play::{clientbound as s2c, serverbound as c2s};

    let _online_guard = OnlinePlayerGuard::acquire(&state.online_players);
    let entity_id = state.next_entity_id.fetch_add(1, Ordering::Relaxed);

    // Determine index of "minecraft:overworld" inside synchronized dimension_type registry
    let overworld_dim_id = generated_registries::SYNCHRONIZED_REGISTRIES
        .iter()
        .find(|&&(reg, _)| reg == "minecraft:dimension_type")
        .and_then(|(_, entries)| entries.iter().position(|&e| e == "minecraft:overworld"))
        .unwrap_or(0) as i32;

    let radius = i32::from(state.props.view_distance.clamp(2, 6));
    let login_pkt = PlayLoginS2c {
        entity_id,
        hardcore: false,
        dimension_names: vec!["minecraft:overworld".into()],
        max_players: state.props.max_players as i32,
        view_distance: radius,
        simulation_distance: radius,
        reduced_debug_info: false,
        show_death_screen: true,
        do_limited_crafting: false,
        dimension_type_id: overworld_dim_id,
        dimension_name: "minecraft:overworld".into(),
        hashed_seed: 0x501A_2A26_3000_0777,
        game_mode: GameMode::from_name(&state.props.gamemode),
        is_debug: false,
        is_flat: true,
        portal_cooldown: 0,
        sea_level: -60,
        online_mode: state.props.online_mode,
        enforces_secure_chat: false,
    };
    conn.send_packet(s2c::LOGIN, &login_pkt.encode()).await?;

    conn.send_packet(
        s2c::SET_CHUNK_CACHE_CENTER,
        &encode_set_chunk_cache_center(0, 0),
    )
    .await?;
    conn.send_packet(
        s2c::GAME_EVENT,
        &encode_game_event(GAME_EVENT_START_WAITING_CHUNKS, 0.0),
    )
    .await?;

    for chunk_bytes in &state.spawn_chunk_packets {
        conn.send_packet(s2c::LEVEL_CHUNK_WITH_LIGHT, chunk_bytes)
            .await?;
    }

    let spawn_pos = [8.0, -58.0, 8.0];
    conn.send_packet(
        s2c::PLAYER_POSITION,
        &encode_player_position(1, spawn_pos, 0.0, 0.0),
    )
    .await?;

    info!(
        %username,
        %uuid,
        entity_id,
        chunks = state.spawn_chunk_packets.len(),
        "player spawned in Solara-Sky world"
    );

    let mut keep_alive_tick = tokio::time::interval(Duration::from_secs(10));
    // Consume immediate first tick
    keep_alive_tick.tick().await;
    let mut player_pos = spawn_pos;

    loop {
        tokio::select! {
            _ = keep_alive_tick.tick() => {
                let id = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64;
                let mut w = PacketWriter::with_capacity(8);
                w.write_i64(id);
                conn.send_packet(s2c::KEEP_ALIVE, w.as_slice()).await?;
            }
            pkt_res = conn.read_packet() => {
                let Some((pkt_id, payload)) = pkt_res? else {
                    info!(%username, "player disconnected cleanly");
                    return Ok(());
                };
                match pkt_id {
                    c2s::ACCEPT_TELEPORTATION => {
                        let mut r = PacketReader::new(&payload);
                        let ack = AcceptTeleportC2s::decode(&mut r)?;
                        player_pos = [ack.x, ack.y, ack.z];
                    }
                    c2s::MOVE_PLAYER_POS => {
                        let mut r = PacketReader::new(&payload);
                        player_pos = [r.read_f64()?, r.read_f64()?, r.read_f64()?];
                    }
                    c2s::MOVE_PLAYER_POS_ROT => {
                        let mut r = PacketReader::new(&payload);
                        player_pos = [r.read_f64()?, r.read_f64()?, r.read_f64()?];
                    }
                    c2s::KEEP_ALIVE
                    | c2s::MOVE_PLAYER_ROT
                    | c2s::MOVE_PLAYER_STATUS_ONLY
                    | c2s::CLIENT_TICK_END
                    | c2s::PLAYER_LOADED
                    | c2s::CHUNK_BATCH_RECEIVED
                    | c2s::CLIENT_INFORMATION
                    | c2s::CUSTOM_PAYLOAD => {}
                    other => {
                        debug!(%username, pkt_id = other, pos = ?player_pos, "ignoring unhandled Play packet");
                    }
                }
            }
        }
    }
}
