use crate::admin::AccessControl;
use crate::config::{ProxyMode, ServerProperties, SkyConfig};
use crate::tick::TickProfiler;
use crate::world_state::{BroadcastEvent, ConnectedPlayer, LiveWorld, WORLD_SEED};
use anyhow::{Context, Result, bail};
use rand::RngCore;
use rsa::pkcs8::EncodePublicKey;
use rsa::{Pkcs1v15Encrypt, RsaPrivateKey};
use serde::Deserialize;
use sky_api::{
    BlockChangeEvent, BlockChangeKind, ChatEvent, CommandEvent, PlayerPreLoginEvent, PluginManager,
    StatusPingEvent,
};
use sky_command::complete_command_input;
use sky_entity::{MobKind, PlayerVitals, WorldItemDrop, block_survival_drop};
use sky_inventory::recipes::CORE_RECIPES;
use sky_inventory::{ItemStack, PlayerInventory};
use sky_protocol::buf::{PacketReader, PacketWriter};
use sky_protocol::config_play::{
    AcceptTeleportC2s, GAME_EVENT_CHANGE_GAME_MODE, GAME_EVENT_START_WAITING_CHUNKS, GameMode,
    KnownPack, PlayLoginS2c, PlayerActionC2s, UseItemOnC2s, decode_chat_command,
    decode_chat_message, decode_select_known_packs, direction_offset, encode_block_changed_ack,
    encode_block_update, encode_brand_payload, encode_enabled_features, encode_game_event,
    encode_nbt_disconnect, encode_player_position, encode_registry_data_known,
    encode_select_known_packs, encode_set_chunk_cache_center, encode_system_chat,
    encode_update_tags,
};
use sky_protocol::crypto::{MinecraftCipher, mojang_server_hash};
use sky_protocol::entities::{
    PlayerInfoEntry, encode_add_entity, encode_add_player_entity, encode_player_info_add,
    encode_player_info_remove, encode_remove_entities, encode_teleport_entity,
};
use sky_protocol::error::ProtocolError;
use sky_protocol::frame::{MAX_WIRE_FRAME_BYTES, decode_frame_body, encode_frame};
use sky_protocol::packets::{
    ClientIntent, HandshakeC2s, LoginCompressionS2c, LoginCustomQueryAnswerC2s,
    LoginCustomQueryS2c, LoginDisconnectS2c, LoginFinishedS2c, LoginHelloC2s, LoginHelloS2c,
    LoginKeyC2s, ProfileProperty, ServerStatusJson, offline_player_uuid, parse_bungeecord_address,
};
use sky_protocol::survival::{
    CLIENT_CMD_PERFORM_RESPAWN, decode_attack_entity, decode_client_command,
    decode_command_suggestion, decode_use_item, encode_animate, encode_command_suggestions,
    encode_hurt_animation, encode_respawn, encode_set_health, encode_set_player_inventory_slot,
    encode_set_time_26_3,
};
use sky_protocol::velocity::{
    VELOCITY_MAX_SUPPORTED_VERSION, VELOCITY_PLAYER_INFO_CHANNEL, verify_velocity_player_info,
};
use sky_protocol::{generated_packets, generated_registries};
use sky_world::{ChunkPos, generated_blocks};
use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tracing::{debug, info, warn};
use uuid::Uuid;

const HOTBAR_BLOCKS: [u32; 9] = [
    generated_blocks::default_state::STONE,
    generated_blocks::default_state::COBBLESTONE,
    generated_blocks::default_state::OAK_PLANKS,
    generated_blocks::default_state::OAK_LOG,
    generated_blocks::default_state::DIRT,
    generated_blocks::default_state::GRASS_BLOCK,
    generated_blocks::default_state::SAND,
    generated_blocks::default_state::GLASS,
    generated_blocks::default_state::GLOWSTONE,
];

pub struct SharedServerState {
    pub props: ServerProperties,
    pub sky_cfg: SkyConfig,
    pub rsa_key: Option<(RsaPrivateKey, Vec<u8>)>,
    pub http: reqwest::Client,
    pub plugins: PluginManager,
    pub acl: AccessControl,
    pub profiler: TickProfiler,
    pub online_players: AtomicUsize,
    pub next_entity_id: AtomicI32,
    pub world: LiveWorld,
}

impl SharedServerState {
    pub fn new(
        config_dir: &Path,
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

        let acl = AccessControl::load_or_create(config_dir)?;
        let profiler = TickProfiler::new(sky_cfg.performance.target_tps);
        let radius = i32::from(props.view_distance.clamp(2, 16));
        let region_dir = config_dir.join("world").join("region");
        let world = LiveWorld::load_or_generate(region_dir, radius.min(6))?;

        Ok(Self {
            props,
            sky_cfg,
            rsa_key,
            http,
            plugins,
            acl,
            profiler,
            online_players: AtomicUsize::new(0),
            next_entity_id: AtomicI32::new(100),
            world,
        })
    }
}

struct OnlinePlayerGuard<'a> {
    counter: &'a AtomicUsize,
    world: &'a LiveWorld,
    entity_id: i32,
    uuid: Uuid,
}

impl<'a> OnlinePlayerGuard<'a> {
    fn acquire(counter: &'a AtomicUsize, world: &'a LiveWorld, player: ConnectedPlayer) -> Self {
        let entity_id = player.entity_id;
        let uuid = player.uuid;
        counter.fetch_add(1, Ordering::Relaxed);
        world.add_player(player);
        Self {
            counter,
            world,
            entity_id,
            uuid,
        }
    }
}

impl Drop for OnlinePlayerGuard<'_> {
    fn drop(&mut self) {
        use generated_packets::play::clientbound as s2c;
        self.world.remove_player(self.entity_id);
        self.counter.fetch_sub(1, Ordering::Relaxed);
        self.world.broadcast_others(
            self.entity_id,
            s2c::REMOVE_ENTITIES,
            encode_remove_entities(&[self.entity_id]),
        );
        self.world.broadcast_others(
            self.entity_id,
            s2c::PLAYER_INFO_REMOVE,
            encode_player_info_remove(&[self.uuid]),
        );
    }
}

struct FramedConnection {
    stream: TcpStream,
    cipher: Option<MinecraftCipher>,
    compression_threshold: Option<usize>,
    max_packet_bytes: usize,
    rate_limit_per_sec: u32,
    rate_window_start: Instant,
    packets_in_window: u32,
}

impl FramedConnection {
    fn new(stream: TcpStream, max_packet_bytes: usize, rate_limit_per_sec: u32) -> Self {
        Self {
            stream,
            cipher: None,
            compression_threshold: None,
            max_packet_bytes: max_packet_bytes.clamp(4096, MAX_WIRE_FRAME_BYTES),
            rate_limit_per_sec,
            rate_window_start: Instant::now(),
            packets_in_window: 0,
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
        if frame_len > self.max_packet_bytes {
            return Err(ProtocolError::PacketTooLarge {
                len: frame_len,
                max: self.max_packet_bytes,
            });
        }

        if self.rate_limit_per_sec > 0 {
            let now = Instant::now();
            if now.duration_since(self.rate_window_start) >= Duration::from_secs(1) {
                self.rate_window_start = now;
                self.packets_in_window = 0;
            }
            self.packets_in_window += 1;
            if self.packets_in_window > self.rate_limit_per_sec {
                return Err(ProtocolError::PacketTooLarge {
                    len: self.packets_in_window as usize,
                    max: self.rate_limit_per_sec as usize,
                });
            }
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
    let mut conn = FramedConnection::new(
        stream,
        state.sky_cfg.performance.max_packet_bytes,
        state.sky_cfg.performance.packet_rate_limit_per_sec,
    );

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

    if let Some(ban_msg) = state.acl.ban_reason(&username, uuid) {
        send_login_disconnect(conn, format!("You are banned from this server: {ban_msg}")).await?;
        return Ok(());
    }
    if state.props.white_list && !state.acl.is_whitelisted(&username, uuid) {
        send_login_disconnect(conn, "You are not white-listed on this server!").await?;
        return Ok(());
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
        properties: properties.clone(),
        session_id,
    };
    conn.send_packet(LoginFinishedS2c::PACKET_ID, &finished.encode())
        .await?;

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

    handle_play(conn, &username, uuid, properties, state).await
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
            c2s::CLIENT_INFORMATION | c2s::CUSTOM_PAYLOAD | c2s::RESOURCE_PACK => {}
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

async fn send_chat(conn: &mut FramedConnection, text: &str) -> Result<(), ProtocolError> {
    conn.send_packet(
        generated_packets::play::clientbound::SYSTEM_CHAT,
        &encode_system_chat(text, false),
    )
    .await
}

async fn stream_missing_chunks(
    conn: &mut FramedConnection,
    center: ChunkPos,
    radius: i32,
    loaded_chunks: &mut HashSet<ChunkPos>,
    world: &LiveWorld,
) -> Result<()> {
    use generated_packets::play::clientbound as s2c;
    conn.send_packet(
        s2c::SET_CHUNK_CACHE_CENTER,
        &encode_set_chunk_cache_center(center.x, center.z),
    )
    .await?;

    let mut needed = Vec::new();
    for cz in (center.z - radius)..=(center.z + radius) {
        for cx in (center.x - radius)..=(center.x + radius) {
            let pos = ChunkPos { x: cx, z: cz };
            if loaded_chunks.insert(pos) {
                needed.push(pos);
            }
        }
    }

    if !needed.is_empty() {
        let packets = world.get_or_generate_chunk_packets_batch(&needed).await?;
        for (_pos, pkt) in packets {
            conn.send_packet(s2c::LEVEL_CHUNK_WITH_LIGHT, &pkt).await?;
        }
    }
    Ok(())
}

async fn sync_player_inventory(conn: &mut FramedConnection, inv: &PlayerInventory) -> Result<()> {
    use generated_packets::play::clientbound as s2c;
    for (slot_idx, st) in inv.main.iter().enumerate() {
        conn.send_packet(
            s2c::SET_PLAYER_INVENTORY,
            &encode_set_player_inventory_slot(slot_idx, st.item_id, st.count),
        )
        .await?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn execute_command(
    cmd_line: &str,
    conn: &mut FramedConnection,
    sender_username: &str,
    sender_uuid: Uuid,
    entity_id: i32,
    player_pos: &mut [f64; 3],
    game_mode: &mut GameMode,
    next_tp_id: &mut i32,
    state: &SharedServerState,
) -> Result<()> {
    use generated_packets::play::clientbound as s2c;

    let parts: Vec<&str> = cmd_line
        .trim()
        .trim_start_matches('/')
        .split_whitespace()
        .collect();
    let Some(&cmd) = parts.first() else {
        return Ok(());
    };
    let lower_cmd = cmd.to_ascii_lowercase();
    let is_op =
        state.acl.is_op(sender_username) || (!state.props.online_mode && !state.acl.has_any_ops());

    // 1. Give registered SkyPlugins first chance to handle or extend commands!
    let mut plugin_cmd = CommandEvent {
        sender_username: sender_username.to_owned(),
        sender_uuid,
        is_op,
        command: lower_cmd.clone(),
        args: parts[1..].iter().map(|&s| s.to_owned()).collect(),
        response_lines: Vec::new(),
        handled: false,
    };
    state.plugins.fire_command(&mut plugin_cmd);
    for line in &plugin_cmd.response_lines {
        send_chat(conn, line).await?;
    }
    if plugin_cmd.handled {
        return Ok(());
    }

    match lower_cmd.as_str() {
        "help" => {
            send_chat(
                conn,
                "Solara-Sky commands: /help, /rules, /guide, /recipes, /craft, /inv, /time, /list, /tps, /plugins, /seed, /gamemode, /tp, /setblock, /heal, /feed, /kill, /spawnmob, /give, /clear, /save",
            )
            .await?;
        }
        "heal" => {
            if let Some(v) = state.world.mutate_player(entity_id, |p| {
                p.vitals.heal_full();
                p.vitals
            }) {
                conn.send_packet(
                    s2c::SET_HEALTH,
                    &encode_set_health(v.health, v.food, v.saturation),
                )
                .await?;
                send_chat(conn, "Restored health to 20.0 HP.").await?;
            }
        }
        "feed" => {
            if let Some(v) = state.world.mutate_player(entity_id, |p| {
                p.vitals.feed_full();
                p.vitals
            }) {
                conn.send_packet(
                    s2c::SET_HEALTH,
                    &encode_set_health(v.health, v.food, v.saturation),
                )
                .await?;
                send_chat(conn, "Restored hunger to 20.").await?;
            }
        }
        "kill" => {
            if let Some(v) = state.world.mutate_player(entity_id, |p| {
                p.vitals.health = 0.0;
                p.vitals
            }) {
                conn.send_packet(s2c::SET_HEALTH, &encode_set_health(0.0, v.food, 0.0))
                    .await?;
                state.world.broadcast_all(
                    s2c::SYSTEM_CHAT,
                    encode_system_chat(&format!("{sender_username} fell out of the world"), false),
                );
            }
        }
        "spawnmob" | "summon" => {
            let Some(&kind_str) = parts.get(1) else {
                send_chat(conn, "Usage: /spawnmob <pig|cow|sheep|zombie>").await?;
                return Ok(());
            };
            let Some(kind) = MobKind::from_name(kind_str) else {
                send_chat(conn, "Supported mobs: pig, cow, sheep, zombie").await?;
                return Ok(());
            };
            let mob_id = state.next_entity_id.fetch_add(1, Ordering::Relaxed);
            let spawn_at = [player_pos[0] + 2.0, player_pos[1], player_pos[2] + 2.0];
            let mob = state.world.spawn_mob(mob_id, kind, spawn_at);
            state.world.broadcast_all(
                s2c::ADD_ENTITY,
                encode_add_entity(
                    mob.entity_id,
                    mob.uuid,
                    mob.kind.protocol_entity_type(),
                    mob.position,
                    mob.yaw_deg,
                    0.0,
                ),
            );
            send_chat(
                conn,
                &format!(
                    "Spawned {:?} (#{}) at ({:.1}, {:.1}, {:.1}).",
                    kind, mob.entity_id, spawn_at[0], spawn_at[1], spawn_at[2]
                ),
            )
            .await?;
        }
        "give" => {
            let Some(&item_name) = parts.get(1) else {
                send_chat(conn, "Usage: /give <item> [count]").await?;
                return Ok(());
            };
            let count: u8 = parts
                .get(2)
                .and_then(|s| s.parse().ok())
                .unwrap_or(1)
                .clamp(1, 64);
            if let Some(stack) = ItemStack::from_key(item_name, count) {
                if let Some(inv) = state.world.mutate_player(entity_id, |p| {
                    let _ = p.inventory.insert(stack);
                    p.inventory.clone()
                }) {
                    sync_player_inventory(conn, &inv).await?;
                }
                send_chat(
                    conn,
                    &format!("Gave {count}x {item_name} to {sender_username}."),
                )
                .await?;
            } else {
                send_chat(conn, &format!("Unknown item '{item_name}'.")).await?;
            }
        }
        "clear" => {
            if let Some(inv) = state.world.mutate_player(entity_id, |p| {
                p.inventory.clear();
                p.inventory.clone()
            }) {
                sync_player_inventory(conn, &inv).await?;
            }
            send_chat(conn, "Cleared player inventory.").await?;
        }
        "craft" => {
            let Some(&recipe_name) = parts.get(1) else {
                send_chat(conn, "Usage: /craft <recipe> (see /recipes)").await?;
                return Ok(());
            };
            let res = state
                .world
                .mutate_player(entity_id, |p| {
                    p.inventory
                        .craft_recipe(recipe_name)
                        .map(|out| (out, p.inventory.clone()))
                })
                .unwrap_or(Err("Player not found."));
            match res {
                Ok((crafted, inv)) => {
                    sync_player_inventory(conn, &inv).await?;
                    send_chat(
                        conn,
                        &format!(
                            "Crafted {}x {}!",
                            crafted.count,
                            crafted.key().unwrap_or(recipe_name)
                        ),
                    )
                    .await?;
                }
                Err(msg) => {
                    send_chat(conn, msg).await?;
                }
            }
        }
        "recipes" => {
            let names: Vec<&str> = CORE_RECIPES.iter().map(|r| r.name).collect();
            send_chat(
                conn,
                &format!("Craftable ({}): {}", names.len(), names.join(", ")),
            )
            .await?;
        }
        "inv" | "inventory" => {
            if let Some(summary) = state.world.mutate_player(entity_id, |p| {
                let items: Vec<String> = p
                    .inventory
                    .main
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| !s.is_empty())
                    .map(|(idx, s)| {
                        let short = s
                            .key()
                            .unwrap_or("unknown")
                            .trim_start_matches("minecraft:");
                        format!("[{idx}] {}x {short}", s.count)
                    })
                    .collect();
                if items.is_empty() {
                    "Inventory is empty.".to_owned()
                } else {
                    format!("Inventory: {}", items.join(", "))
                }
            }) {
                send_chat(conn, &summary).await?;
            }
        }
        "time" => {
            let token = match (parts.get(1).copied(), parts.get(2).copied()) {
                (Some("set"), Some(val)) => Some(val),
                (Some(val), _) => Some(val),
                _ => None,
            };
            let Some(val) = token else {
                let (_, day_t) = state.world.clock_snapshot();
                send_chat(
                    conn,
                    &format!("Day time: {day_t} ticks. Usage: /time <day|noon|sunset|night|ticks>"),
                )
                .await?;
                return Ok(());
            };
            let parsed_ticks = match val.to_ascii_lowercase().as_str() {
                "day" | "morning" => Some(1_000i64),
                "noon" | "midday" => Some(6_000),
                "sunset" | "dusk" => Some(12_500),
                "night" | "midnight" => Some(18_000),
                other => other.parse::<i64>().ok(),
            };
            if let Some(ticks) = parsed_ticks {
                let (gt, dt) = state.world.set_day_time(ticks);
                state
                    .world
                    .broadcast_all(s2c::SET_TIME, encode_set_time_26_3(gt, dt, 1.0));
                send_chat(conn, &format!("Set Overworld time to {dt} ticks.")).await?;
            } else {
                send_chat(conn, "Usage: /time <day|noon|sunset|night|ticks>").await?;
            }
        }
        "tps" | "mspt" => {
            let r = state.profiler.snapshot();
            send_chat(
                conn,
                &format!(
                    "Solara-Sky TPS (5s/1m): {:.1}/{:.1} | MSPT (last/avg/p95): {:.2}/{:.2}/{:.2} ms | FoliaRegions: {} | Chunks: {} | Cores: {} | Tick: #{}",
                    r.tps_5s,
                    r.tps_1m,
                    r.last_mspt,
                    r.avg_mspt_5s,
                    r.p95_mspt_5s,
                    r.active_regions,
                    r.loaded_chunks,
                    r.worker_threads,
                    r.tick_number
                ),
            )
            .await?;
        }
        "plugins" | "pl" => {
            let metas = state.plugins.plugin_metas();
            let summary = if metas.is_empty() {
                "none".to_owned()
            } else {
                metas
                    .iter()
                    .map(|m| format!("{} v{}", m.name, m.version))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            send_chat(conn, &format!("Plugins ({}): {summary}", metas.len())).await?;
        }
        "seed" => {
            send_chat(conn, &format!("Seed: [{WORLD_SEED}]")).await?;
        }
        "list" => {
            let names = state.world.online_usernames();
            send_chat(
                conn,
                &format!(
                    "Online ({}/{}): {}",
                    names.len(),
                    state.props.max_players,
                    names.join(", ")
                ),
            )
            .await?;
        }
        "op" => {
            if !is_op {
                send_chat(conn, "You do not have operator permission.").await?;
                return Ok(());
            }
            let Some(&target) = parts.get(1) else {
                send_chat(conn, "Usage: /op <player>").await?;
                return Ok(());
            };
            let uid = state
                .world
                .find_online_player(target)
                .map_or_else(|| offline_player_uuid(target), |p| p.uuid);
            state.acl.grant_op(target, uid)?;
            send_chat(conn, &format!("Made {target} a server operator.")).await?;
        }
        "deop" => {
            if !is_op {
                send_chat(conn, "You do not have operator permission.").await?;
                return Ok(());
            }
            let Some(&target) = parts.get(1) else {
                send_chat(conn, "Usage: /deop <player>").await?;
                return Ok(());
            };
            if state.acl.revoke_op(target)? {
                send_chat(conn, &format!("Made {target} no longer a server operator.")).await?;
            } else {
                send_chat(conn, &format!("{target} is not an operator.")).await?;
            }
        }
        "kick" => {
            if !is_op {
                send_chat(conn, "You do not have operator permission.").await?;
                return Ok(());
            }
            let Some(&target) = parts.get(1) else {
                send_chat(conn, "Usage: /kick <player> [reason]").await?;
                return Ok(());
            };
            let reason = if parts.len() > 2 {
                parts[2..].join(" ")
            } else {
                "Kicked by an operator.".to_owned()
            };
            state.world.kick_player(target, &reason);
            send_chat(conn, &format!("Kicked {target}: {reason}")).await?;
        }
        "ban" => {
            if !is_op {
                send_chat(conn, "You do not have operator permission.").await?;
                return Ok(());
            }
            let Some(&target) = parts.get(1) else {
                send_chat(conn, "Usage: /ban <player> [reason]").await?;
                return Ok(());
            };
            let reason = if parts.len() > 2 {
                parts[2..].join(" ")
            } else {
                "Banned by an operator.".to_owned()
            };
            let uid = state
                .world
                .find_online_player(target)
                .map_or_else(|| offline_player_uuid(target), |p| p.uuid);
            state.acl.ban_player(target, uid, &reason)?;
            state.world.kick_player(target, format!("Banned: {reason}"));
            send_chat(conn, &format!("Banned {target}: {reason}")).await?;
        }
        "pardon" => {
            if !is_op {
                send_chat(conn, "You do not have operator permission.").await?;
                return Ok(());
            }
            let Some(&target) = parts.get(1) else {
                send_chat(conn, "Usage: /pardon <player>").await?;
                return Ok(());
            };
            if state.acl.pardon_player(target)? {
                send_chat(conn, &format!("Unbanned {target}.")).await?;
            } else {
                send_chat(conn, &format!("{target} was not banned.")).await?;
            }
        }
        "whitelist" => {
            if !is_op {
                send_chat(conn, "You do not have operator permission.").await?;
                return Ok(());
            }
            if parts.len() < 3 {
                send_chat(conn, "Usage: /whitelist <add|remove> <player>").await?;
                return Ok(());
            }
            let target = parts[2];
            match parts[1].to_ascii_lowercase().as_str() {
                "add" => {
                    let uid = state
                        .world
                        .find_online_player(target)
                        .map_or_else(|| offline_player_uuid(target), |p| p.uuid);
                    state.acl.add_whitelist(target, uid)?;
                    send_chat(conn, &format!("Added {target} to the whitelist.")).await?;
                }
                "remove" => {
                    state.acl.remove_whitelist(target)?;
                    send_chat(conn, &format!("Removed {target} from the whitelist.")).await?;
                }
                _ => send_chat(conn, "Usage: /whitelist <add|remove> <player>").await?,
            }
        }
        "gamemode" | "gm" => {
            let Some(&arg) = parts.get(1) else {
                send_chat(
                    conn,
                    "Usage: /gamemode <survival|creative|adventure|spectator>",
                )
                .await?;
                return Ok(());
            };
            let new_mode = GameMode::from_name(arg);
            *game_mode = new_mode;
            state.world.update_player_gamemode(entity_id, new_mode);
            conn.send_packet(
                s2c::GAME_EVENT,
                &encode_game_event(GAME_EVENT_CHANGE_GAME_MODE, (new_mode as u8) as f32),
            )
            .await?;
            send_chat(conn, &format!("Set game mode to {new_mode:?}.")).await?;
        }
        "tp" => {
            if parts.len() < 4 {
                send_chat(conn, "Usage: /tp <x> <y> <z>").await?;
                return Ok(());
            }
            if let (Ok(x), Ok(y), Ok(z)) = (
                parts[1].parse::<f64>(),
                parts[2].parse::<f64>(),
                parts[3].parse::<f64>(),
            ) {
                *player_pos = [x, y, z];
                state
                    .world
                    .update_player_transform(entity_id, *player_pos, 0.0, 0.0);
                *next_tp_id += 1;
                conn.send_packet(
                    s2c::PLAYER_POSITION,
                    &encode_player_position(*next_tp_id, *player_pos, 0.0, 0.0),
                )
                .await?;
                state.world.broadcast_others(
                    entity_id,
                    s2c::TELEPORT_ENTITY,
                    encode_teleport_entity(entity_id, *player_pos, 0.0, 0.0, false),
                );
                send_chat(conn, &format!("Teleported to ({x:.1}, {y:.1}, {z:.1}).")).await?;
            } else {
                send_chat(conn, "Invalid coordinates for /tp.").await?;
            }
        }
        "setblock" => {
            if parts.len() < 5 {
                send_chat(conn, "Usage: /setblock <x> <y> <z> <block>").await?;
                return Ok(());
            }
            if let (Ok(x), Ok(y), Ok(z)) = (
                parts[1].parse::<i32>(),
                parts[2].parse::<i32>(),
                parts[3].parse::<i32>(),
            ) {
                let raw_name = parts[4];
                let full_name = if raw_name.contains(':') {
                    raw_name.to_owned()
                } else {
                    format!("minecraft:{raw_name}")
                };
                if let Some(st) = generated_blocks::default_state_by_name(&full_name) {
                    if state.world.set_block(x, y, z, st) {
                        state
                            .world
                            .broadcast_all(s2c::BLOCK_UPDATE, encode_block_update(x, y, z, st));
                        send_chat(conn, &format!("Placed {full_name} at ({x}, {y}, {z}).")).await?;
                    } else {
                        send_chat(conn, "Y coordinate must be within -64..319.").await?;
                    }
                } else {
                    send_chat(conn, &format!("Unknown block '{full_name}'.")).await?;
                }
            }
        }
        "save" => match state.world.save_dirty_chunks(true).await {
            Ok(saved) => {
                send_chat(
                    conn,
                    &format!("Saved {saved} chunks to Anvil .mca region files."),
                )
                .await?;
            }
            Err(err) => {
                warn!(error = %err, "Anvil save command failed");
                send_chat(conn, "World save failed — check server logs.").await?;
            }
        },
        other => {
            send_chat(
                conn,
                &format!("Unknown command '/{other}'. Type /help for commands."),
            )
            .await?;
        }
    }
    Ok(())
}

async fn handle_play(
    conn: &mut FramedConnection,
    username: &str,
    uuid: Uuid,
    properties: Vec<ProfileProperty>,
    state: &SharedServerState,
) -> Result<()> {
    use generated_packets::play::{clientbound as s2c, serverbound as c2s};

    let entity_id = state.next_entity_id.fetch_add(1, Ordering::Relaxed);
    let mut game_mode = GameMode::from_name(&state.props.gamemode);
    let spawn_pos = [8.0, -58.0, 8.0];

    let existing_players = state.world.snapshot_players();
    let existing_mobs = state.world.snapshot_mobs();
    let existing_drops = state.world.snapshot_item_drops();
    let mut broadcast_rx = state.world.subscribe();

    let starter_inv = PlayerInventory::new_starter_kit();
    let self_player = ConnectedPlayer {
        entity_id,
        username: username.to_owned(),
        uuid,
        properties: properties.clone(),
        position: spawn_pos,
        yaw_deg: 0.0,
        pitch_deg: 0.0,
        game_mode,
        vitals: PlayerVitals::new_full(spawn_pos[1]),
        inventory: starter_inv.clone(),
    };
    let _online_guard =
        OnlinePlayerGuard::acquire(&state.online_players, &state.world, self_player);

    let overworld_dim_id = generated_registries::SYNCHRONIZED_REGISTRIES
        .iter()
        .find(|&&(reg, _)| reg == "minecraft:dimension_type")
        .and_then(|(_, entries)| entries.iter().position(|&e| e == "minecraft:overworld"))
        .unwrap_or(0) as i32;

    let view_radius = i32::from(state.props.view_distance.clamp(2, 16));
    let sim_radius = i32::from(state.props.simulation_distance.clamp(2, 16));

    let login_pkt = PlayLoginS2c {
        entity_id,
        hardcore: false,
        dimension_names: vec!["minecraft:overworld".into()],
        max_players: state.props.max_players as i32,
        view_distance: view_radius,
        simulation_distance: sim_radius,
        reduced_debug_info: false,
        show_death_screen: true,
        do_limited_crafting: false,
        dimension_type_id: overworld_dim_id,
        dimension_name: "minecraft:overworld".into(),
        hashed_seed: WORLD_SEED as i64,
        game_mode,
        is_debug: false,
        is_flat: false,
        portal_cooldown: 0,
        sea_level: -56,
        online_mode: state.props.online_mode,
        enforces_secure_chat: false,
    };
    conn.send_packet(s2c::LOGIN, &login_pkt.encode()).await?;

    conn.send_packet(
        s2c::GAME_EVENT,
        &encode_game_event(GAME_EVENT_START_WAITING_CHUNKS, 0.0),
    )
    .await?;

    let mut current_center = ChunkPos { x: 0, z: 0 };
    let mut loaded_chunks = HashSet::new();
    stream_missing_chunks(
        conn,
        current_center,
        view_radius,
        &mut loaded_chunks,
        &state.world,
    )
    .await?;

    // Sync tab-list entries (self + existing players) and spawn existing visible player entities
    let mut tab_entries: Vec<PlayerInfoEntry> = existing_players
        .iter()
        .map(|p| PlayerInfoEntry {
            uuid: p.uuid,
            username: p.username.clone(),
            properties: p.properties.clone(),
            game_mode: p.game_mode,
            latency_ms: 10,
        })
        .collect();
    let self_info = PlayerInfoEntry {
        uuid,
        username: username.to_owned(),
        properties,
        game_mode,
        latency_ms: 5,
    };
    tab_entries.push(self_info.clone());
    conn.send_packet(
        s2c::PLAYER_INFO_UPDATE,
        &encode_player_info_add(&tab_entries),
    )
    .await?;

    for other in &existing_players {
        conn.send_packet(
            s2c::ADD_ENTITY,
            &encode_add_player_entity(
                other.entity_id,
                other.uuid,
                other.position,
                other.yaw_deg,
                other.pitch_deg,
            ),
        )
        .await?;
    }

    for mob in &existing_mobs {
        conn.send_packet(
            s2c::ADD_ENTITY,
            &encode_add_entity(
                mob.entity_id,
                mob.uuid,
                mob.kind.protocol_entity_type(),
                mob.position,
                mob.yaw_deg,
                0.0,
            ),
        )
        .await?;
    }

    for drop in &existing_drops {
        conn.send_packet(
            s2c::ADD_ENTITY,
            &encode_add_entity(
                drop.entity_id,
                drop.uuid,
                WorldItemDrop::PROTOCOL_ENTITY_TYPE,
                drop.position,
                0.0,
                0.0,
            ),
        )
        .await?;
    }

    let (gt, dt) = state.world.clock_snapshot();
    conn.send_packet(s2c::SET_TIME, &encode_set_time_26_3(gt, dt, 1.0))
        .await?;
    sync_player_inventory(conn, &starter_inv).await?;

    conn.send_packet(s2c::SET_HEALTH, &encode_set_health(20.0, 20, 5.0))
        .await?;

    // Broadcast this new player's tab entry + visible 3D entity to all other online players!
    state.world.broadcast_others(
        entity_id,
        s2c::PLAYER_INFO_UPDATE,
        encode_player_info_add(&[self_info]),
    );
    state.world.broadcast_others(
        entity_id,
        s2c::ADD_ENTITY,
        encode_add_player_entity(entity_id, uuid, spawn_pos, 0.0, 0.0),
    );

    let mut next_tp_id = 1i32;
    conn.send_packet(
        s2c::PLAYER_POSITION,
        &encode_player_position(next_tp_id, spawn_pos, 0.0, 0.0),
    )
    .await?;

    state.world.broadcast_all(
        s2c::SYSTEM_CHAT,
        encode_system_chat(&format!("{username} joined the game"), false),
    );
    send_chat(
        conn,
        "Welcome to Solara-Sky (MC 26.3)! Procedural terrain, lighting & multiplayer entities active.",
    )
    .await?;

    info!(
        %username,
        %uuid,
        entity_id,
        chunks = loaded_chunks.len(),
        "player spawned in Solara-Sky procedural world"
    );

    let mut keep_alive_tick = tokio::time::interval(Duration::from_secs(10));
    keep_alive_tick.tick().await;
    let mut player_pos = spawn_pos;
    let mut player_yaw = 0.0f32;
    let mut player_pitch = 0.0f32;
    let mut held_slot: usize = 0;

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
            bcast = broadcast_rx.recv() => {
                match bcast {
                    Ok(BroadcastEvent::Packet { exclude_entity_id, packet_id, payload })
                        if exclude_entity_id != Some(entity_id) =>
                    {
                        conn.send_packet(packet_id, &payload).await?;
                    }
                    Ok(BroadcastEvent::DirectPlayer { target_entity_id, packet_id, payload })
                        if target_entity_id == entity_id =>
                    {
                        conn.send_packet(packet_id, &payload).await?;
                    }
                    Ok(BroadcastEvent::Kick { target_username_lower, reason })
                        if target_username_lower == username.to_ascii_lowercase() =>
                    {
                        conn.send_packet(s2c::DISCONNECT, &encode_nbt_disconnect(&reason)).await?;
                        return Ok(());
                    }
                    _ => {}
                }
            }
            pkt_res = conn.read_packet() => {
                let Some((pkt_id, payload)) = pkt_res? else {
                    info!(%username, "player disconnected cleanly");
                    state.world.broadcast_all(
                        s2c::SYSTEM_CHAT,
                        encode_system_chat(&format!("{username} left the game"), false),
                    );
                    return Ok(());
                };
                match pkt_id {
                    c2s::ACCEPT_TELEPORTATION => {
                        let mut r = PacketReader::new(&payload);
                        let ack = AcceptTeleportC2s::decode(&mut r)?;
                        player_pos = [ack.x, ack.y, ack.z];
                        player_yaw = ack.yaw;
                        player_pitch = ack.pitch;
                        state.world.update_player_transform(entity_id, player_pos, player_yaw, player_pitch);
                    }
                    c2s::MOVE_PLAYER_POS => {
                        let mut r = PacketReader::new(&payload);
                        player_pos = [r.read_f64()?, r.read_f64()?, r.read_f64()?];
                        let flags = r.read_u8()?;
                        let on_ground = (flags & 1) != 0;
                        if let Some((vitals, _dmg)) = state.world.record_player_movement(
                            entity_id,
                            player_pos,
                            player_yaw,
                            player_pitch,
                            on_ground,
                        ) {
                            conn.send_packet(
                                s2c::SET_HEALTH,
                                &encode_set_health(vitals.health, vitals.food, vitals.saturation),
                            )
                            .await?;
                            state.world.broadcast_all(
                                s2c::HURT_ANIMATION,
                                encode_hurt_animation(entity_id, 0.0),
                            );
                            if vitals.is_dead() {
                                state.world.broadcast_all(
                                    s2c::SYSTEM_CHAT,
                                    encode_system_chat(&format!("{username} hit the ground too hard"), false),
                                );
                            }
                        }
                        state.world.broadcast_others(
                            entity_id,
                            s2c::TELEPORT_ENTITY,
                            encode_teleport_entity(entity_id, player_pos, player_yaw, player_pitch, on_ground),
                        );
                        let new_center = ChunkPos {
                            x: (player_pos[0].floor() as i32).div_euclid(16),
                            z: (player_pos[2].floor() as i32).div_euclid(16),
                        };
                        if new_center != current_center {
                            current_center = new_center;
                            stream_missing_chunks(conn, current_center, view_radius, &mut loaded_chunks, &state.world).await?;
                        }
                    }
                    c2s::MOVE_PLAYER_POS_ROT => {
                        let mut r = PacketReader::new(&payload);
                        player_pos = [r.read_f64()?, r.read_f64()?, r.read_f64()?];
                        player_yaw = r.read_f32()?;
                        player_pitch = r.read_f32()?;
                        let flags = r.read_u8()?;
                        let on_ground = (flags & 1) != 0;
                        if let Some((vitals, _dmg)) = state.world.record_player_movement(
                            entity_id,
                            player_pos,
                            player_yaw,
                            player_pitch,
                            on_ground,
                        ) {
                            conn.send_packet(
                                s2c::SET_HEALTH,
                                &encode_set_health(vitals.health, vitals.food, vitals.saturation),
                            )
                            .await?;
                            state.world.broadcast_all(
                                s2c::HURT_ANIMATION,
                                encode_hurt_animation(entity_id, 0.0),
                            );
                            if vitals.is_dead() {
                                state.world.broadcast_all(
                                    s2c::SYSTEM_CHAT,
                                    encode_system_chat(&format!("{username} hit the ground too hard"), false),
                                );
                            }
                        }
                        state.world.broadcast_others(
                            entity_id,
                            s2c::TELEPORT_ENTITY,
                            encode_teleport_entity(entity_id, player_pos, player_yaw, player_pitch, on_ground),
                        );
                        let new_center = ChunkPos {
                            x: (player_pos[0].floor() as i32).div_euclid(16),
                            z: (player_pos[2].floor() as i32).div_euclid(16),
                        };
                        if new_center != current_center {
                            current_center = new_center;
                            stream_missing_chunks(conn, current_center, view_radius, &mut loaded_chunks, &state.world).await?;
                        }
                    }
                    c2s::ATTACK => {
                        let mut r = PacketReader::new(&payload);
                        let target_id = decode_attack_entity(&mut r)?;
                        if target_id != entity_id {
                            let weapon_dmg = state
                                .world
                                .mutate_player(entity_id, |p| p.inventory.held_stack().melee_attack_damage())
                                .unwrap_or(4.0);
                            if let Some((mob, hurt_yaw, died)) =
                                state.world.attack_mob(target_id, player_pos, weapon_dmg.max(5.0))
                            {
                                state.world.broadcast_all(
                                    s2c::HURT_ANIMATION,
                                    encode_hurt_animation(mob.entity_id, hurt_yaw),
                                );
                                if died {
                                    state.world.broadcast_all(
                                        s2c::REMOVE_ENTITIES,
                                        encode_remove_entities(&[mob.entity_id]),
                                    );
                                    let (loot_item, loot_cnt) = mob.kind.death_loot();
                                    let drop_eid =
                                        state.next_entity_id.fetch_add(1, Ordering::Relaxed);
                                    let drop = state.world.spawn_item_drop(
                                        drop_eid,
                                        loot_item,
                                        loot_cnt,
                                        mob.position,
                                    );
                                    state.world.broadcast_all(
                                        s2c::ADD_ENTITY,
                                        encode_add_entity(
                                            drop.entity_id,
                                            drop.uuid,
                                            WorldItemDrop::PROTOCOL_ENTITY_TYPE,
                                            drop.position,
                                            0.0,
                                            0.0,
                                        ),
                                    );
                                } else {
                                    state.world.broadcast_all(
                                        s2c::TELEPORT_ENTITY,
                                        encode_teleport_entity(
                                            mob.entity_id,
                                            mob.position,
                                            mob.yaw_deg,
                                            0.0,
                                            true,
                                        ),
                                    );
                                }
                            } else if let Some((victim, _dealt, hurt_yaw)) =
                                state.world.attack_player(target_id, player_pos, weapon_dmg)
                            {
                                state.world.broadcast_all(
                                    s2c::HURT_ANIMATION,
                                    encode_hurt_animation(victim.entity_id, hurt_yaw),
                                );
                                state.world.send_direct(
                                    victim.entity_id,
                                    s2c::SET_HEALTH,
                                    encode_set_health(
                                        victim.vitals.health,
                                        victim.vitals.food,
                                        victim.vitals.saturation,
                                    ),
                                );
                                state.world.send_direct(
                                    victim.entity_id,
                                    s2c::PLAYER_POSITION,
                                    encode_player_position(
                                        9000 + victim.entity_id,
                                        victim.position,
                                        victim.yaw_deg,
                                        victim.pitch_deg,
                                    ),
                                );
                                state.world.broadcast_others(
                                    victim.entity_id,
                                    s2c::TELEPORT_ENTITY,
                                    encode_teleport_entity(
                                        victim.entity_id,
                                        victim.position,
                                        victim.yaw_deg,
                                        victim.pitch_deg,
                                        false,
                                    ),
                                );
                                if victim.vitals.is_dead() {
                                    state.world.broadcast_all(
                                        s2c::SYSTEM_CHAT,
                                        encode_system_chat(
                                            &format!("{} was slain by {username}", victim.username),
                                            false,
                                        ),
                                    );
                                }
                            }
                        }
                    }
                    c2s::CLIENT_COMMAND => {
                        let mut r = PacketReader::new(&payload);
                        if decode_client_command(&mut r)? == CLIENT_CMD_PERFORM_RESPAWN {
                            player_pos = spawn_pos;
                            player_yaw = 0.0;
                            player_pitch = 0.0;
                            let vitals = state
                                .world
                                .mutate_player(entity_id, |p| {
                                    p.position = spawn_pos;
                                    p.vitals.respawn_reset(spawn_pos[1]);
                                    p.vitals
                                })
                                .unwrap_or_else(|| PlayerVitals::new_full(spawn_pos[1]));
                            conn.send_packet(
                                s2c::RESPAWN,
                                &encode_respawn(
                                    overworld_dim_id,
                                    "minecraft:overworld",
                                    WORLD_SEED as i64,
                                    game_mode,
                                    -56,
                                    0,
                                ),
                            )
                            .await?;
                            conn.send_packet(
                                s2c::SET_HEALTH,
                                &encode_set_health(vitals.health, vitals.food, vitals.saturation),
                            )
                            .await?;
                            next_tp_id += 1;
                            conn.send_packet(
                                s2c::PLAYER_POSITION,
                                &encode_player_position(next_tp_id, spawn_pos, 0.0, 0.0),
                            )
                            .await?;
                            state.world.broadcast_others(
                                entity_id,
                                s2c::TELEPORT_ENTITY,
                                encode_teleport_entity(entity_id, spawn_pos, 0.0, 0.0, true),
                            );
                        }
                    }
                    c2s::COMMAND_SUGGESTION => {
                        let mut r = PacketReader::new(&payload);
                        let (tx_id, input) = decode_command_suggestion(&mut r)?;
                        let is_op = state.acl.is_op(username)
                            || (!state.props.online_mode && !state.acl.has_any_ops());
                        let online = state.world.online_usernames();
                        let (start, length, matches) =
                            complete_command_input(&input, is_op, &online);
                        conn.send_packet(
                            s2c::COMMAND_SUGGESTIONS,
                            &encode_command_suggestions(tx_id, start, length, &matches),
                        )
                        .await?;
                    }
                    c2s::MOVE_PLAYER_ROT => {
                        let mut r = PacketReader::new(&payload);
                        player_yaw = r.read_f32()?;
                        player_pitch = r.read_f32()?;
                        let flags = r.read_u8()?;
                        state.world.update_player_transform(entity_id, player_pos, player_yaw, player_pitch);
                        state.world.broadcast_others(
                            entity_id,
                            s2c::TELEPORT_ENTITY,
                            encode_teleport_entity(entity_id, player_pos, player_yaw, player_pitch, (flags & 1) != 0),
                        );
                    }
                    c2s::SET_CARRIED_ITEM => {
                        let mut r = PacketReader::new(&payload);
                        let slot = usize::from(r.read_u16()?);
                        if slot < HOTBAR_BLOCKS.len() {
                            held_slot = slot;
                            state.world.mutate_player(entity_id, |p| {
                                p.inventory.selected_hotbar = slot;
                            });
                        }
                    }
                    c2s::PUNCH => {
                        let mut r = PacketReader::new(&payload);
                        let hand = r.read_varint().unwrap_or(0);
                        let anim = if hand == 0 { 0u8 } else { 3u8 };
                        state
                            .world
                            .broadcast_others(entity_id, s2c::ANIMATE, encode_animate(entity_id, anim));
                    }
                    c2s::USE_ITEM => {
                        let mut r = PacketReader::new(&payload);
                        let (hand, seq) = decode_use_item(&mut r)?;
                        if hand == 0
                            && let Some(Some((vitals, updated_slot))) =
                                state.world.mutate_player(entity_id, |p| {
                                    let held = p.inventory.main[held_slot];
                                    let (pts, sat) = held.food_nutrition()?;
                                    if p.vitals.eat_food(pts, sat) {
                                        let slot_after = p.inventory.consume_slot(held_slot, 1)?;
                                        Some((p.vitals, slot_after))
                                    } else {
                                        None
                                    }
                                })
                        {
                            conn.send_packet(
                                s2c::SET_HEALTH,
                                &encode_set_health(vitals.health, vitals.food, vitals.saturation),
                            )
                            .await?;
                            conn.send_packet(
                                s2c::SET_PLAYER_INVENTORY,
                                &encode_set_player_inventory_slot(
                                    held_slot,
                                    updated_slot.item_id,
                                    updated_slot.count,
                                ),
                            )
                            .await?;
                        }
                        conn.send_packet(s2c::BLOCK_CHANGED_ACK, &encode_block_changed_ack(seq))
                            .await?;
                    }
                    c2s::PLAYER_ACTION => {
                        let mut r = PacketReader::new(&payload);
                        let act = PlayerActionC2s::decode(&mut r)?;
                        let instant_break = game_mode == GameMode::Creative
                            && act.action == PlayerActionC2s::START_DESTROY_BLOCK;
                        let finished_break = act.action == PlayerActionC2s::STOP_DESTROY_BLOCK;
                        if instant_break || finished_break {
                            let (bx, by, bz) = act.pos;
                            let air = generated_blocks::default_state::AIR;
                            let old_state = state.world.get_block(bx, by, bz);
                            let mut bev = BlockChangeEvent {
                                player_username: username.to_owned(),
                                player_uuid: uuid,
                                pos: (bx, by, bz),
                                kind: BlockChangeKind::Break,
                                new_state_id: air,
                                cancelled: false,
                            };
                            state.plugins.fire_block_change(&mut bev);
                            if !bev.cancelled
                                && old_state != air
                                && state.world.set_block(bx, by, bz, air)
                            {
                                state.world.broadcast_all(
                                    s2c::BLOCK_UPDATE,
                                    encode_block_update(bx, by, bz, air),
                                );
                                if game_mode == GameMode::Survival
                                    && let Some(br) = generated_blocks::block_by_state_id(old_state)
                                    && let Some((drop_item, drop_cnt)) =
                                        block_survival_drop(br.name)
                                {
                                    let drop_eid =
                                        state.next_entity_id.fetch_add(1, Ordering::Relaxed);
                                    let drop = state.world.spawn_item_drop(
                                        drop_eid,
                                        drop_item,
                                        drop_cnt,
                                        [bx as f64 + 0.5, by as f64 + 0.25, bz as f64 + 0.5],
                                    );
                                    state.world.broadcast_all(
                                        s2c::ADD_ENTITY,
                                        encode_add_entity(
                                            drop.entity_id,
                                            drop.uuid,
                                            WorldItemDrop::PROTOCOL_ENTITY_TYPE,
                                            drop.position,
                                            0.0,
                                            0.0,
                                        ),
                                    );
                                }
                            }
                        }
                        conn.send_packet(
                            s2c::BLOCK_CHANGED_ACK,
                            &encode_block_changed_ack(act.sequence),
                        )
                        .await?;
                    }
                    c2s::USE_ITEM_ON => {
                        let mut r = PacketReader::new(&payload);
                        let use_pkt = UseItemOnC2s::decode(&mut r)?;
                        if use_pkt.hand == 0 {
                            let (dx, dy, dz) = direction_offset(use_pkt.direction);
                            let (tx, ty, tz) = (
                                use_pkt.pos.0 + dx,
                                use_pkt.pos.1 + dy,
                                use_pkt.pos.2 + dz,
                            );
                            let held_stack = state
                                .world
                                .mutate_player(entity_id, |p| p.inventory.main[held_slot])
                                .unwrap_or(ItemStack::EMPTY);

                            let candidate_block = held_stack
                                .key()
                                .and_then(generated_blocks::default_state_by_name)
                                .or_else(|| {
                                    (game_mode == GameMode::Creative)
                                        .then(|| HOTBAR_BLOCKS[held_slot])
                                });

                            if let Some(placed_state) = candidate_block {
                                let mut bev = BlockChangeEvent {
                                    player_username: username.to_owned(),
                                    player_uuid: uuid,
                                    pos: (tx, ty, tz),
                                    kind: BlockChangeKind::Place,
                                    new_state_id: placed_state,
                                    cancelled: false,
                                };
                                state.plugins.fire_block_change(&mut bev);
                                if !bev.cancelled
                                    && state.world.get_block(tx, ty, tz)
                                        == generated_blocks::default_state::AIR
                                    && state.world.set_block(tx, ty, tz, placed_state)
                                {
                                    state.world.broadcast_all(
                                        s2c::BLOCK_UPDATE,
                                        encode_block_update(tx, ty, tz, placed_state),
                                    );
                                    if game_mode == GameMode::Survival
                                        && let Some(Some(after)) =
                                            state.world.mutate_player(entity_id, |p| {
                                                p.inventory.consume_slot(held_slot, 1)
                                            })
                                    {
                                        conn.send_packet(
                                            s2c::SET_PLAYER_INVENTORY,
                                            &encode_set_player_inventory_slot(
                                                held_slot,
                                                after.item_id,
                                                after.count,
                                            ),
                                        )
                                        .await?;
                                    }
                                }
                            } else if let Some((pts, sat)) = held_stack.food_nutrition()
                                && let Some(Some((vitals, after))) =
                                    state.world.mutate_player(entity_id, |p| {
                                        if p.vitals.eat_food(pts, sat) {
                                            let s = p.inventory.consume_slot(held_slot, 1)?;
                                            Some((p.vitals, s))
                                        } else {
                                            None
                                        }
                                    })
                            {
                                conn.send_packet(
                                    s2c::SET_HEALTH,
                                    &encode_set_health(
                                        vitals.health,
                                        vitals.food,
                                        vitals.saturation,
                                    ),
                                )
                                .await?;
                                conn.send_packet(
                                    s2c::SET_PLAYER_INVENTORY,
                                    &encode_set_player_inventory_slot(
                                        held_slot,
                                        after.item_id,
                                        after.count,
                                    ),
                                )
                                .await?;
                            }
                        }
                        conn.send_packet(
                            s2c::BLOCK_CHANGED_ACK,
                            &encode_block_changed_ack(use_pkt.sequence),
                        )
                        .await?;
                    }
                    c2s::CHAT => {
                        let mut r = PacketReader::new(&payload);
                        let msg = decode_chat_message(&mut r)?;
                        if !msg.trim().is_empty() {
                            let mut cev = ChatEvent {
                                sender_username: username.to_owned(),
                                sender_uuid: uuid,
                                message: msg,
                                cancelled: false,
                            };
                            state.plugins.fire_chat(&mut cev);
                            if !cev.cancelled {
                                info!(%username, message = %cev.message, "chat");
                                state.world.broadcast_all(
                                    s2c::SYSTEM_CHAT,
                                    encode_system_chat(
                                        &format!("<{username}> {}", cev.message),
                                        false,
                                    ),
                                );
                            }
                        }
                    }
                    c2s::CHAT_COMMAND => {
                        let mut r = PacketReader::new(&payload);
                        let cmd = decode_chat_command(&mut r)?;
                        info!(%username, command = %cmd, "command");
                        execute_command(
                            &cmd,
                            conn,
                            username,
                            uuid,
                            entity_id,
                            &mut player_pos,
                            &mut game_mode,
                            &mut next_tp_id,
                            state,
                        )
                        .await?;
                    }
                    c2s::KEEP_ALIVE
                    | c2s::MOVE_PLAYER_STATUS_ONLY
                    | c2s::CLIENT_TICK_END
                    | c2s::PLAYER_LOADED
                    | c2s::CHUNK_BATCH_RECEIVED
                    | c2s::CLIENT_INFORMATION
                    | c2s::CUSTOM_PAYLOAD
                    | c2s::INTERACT => {}
                    other => {
                        debug!(%username, pkt_id = other, "ignoring unhandled Play packet");
                    }
                }
            }
        }
    }
}
