pub mod admin;
pub mod config;
pub mod net;
pub mod tick;
pub mod world_state;

use crate::config::{ServerProperties, SkyConfig};
use crate::net::{SharedServerState, handle_client};
use anyhow::{Context, Result};
use clap::Parser;
use sky_api::{CommandEvent, PluginManager, PluginMeta, ServerInfo, SkyPlugin};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::{info, warn};

/// Built-in core telemetry & server info plugin exercising `sky-api` with zero overhead.
struct SolaraCorePlugin;

impl SkyPlugin for SolaraCorePlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            id: "solara-core".into(),
            name: "SolaraCore".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            authors: vec!["Solara-Inc-Development".into()],
        }
    }

    fn on_command(&self, event: &mut CommandEvent) {
        if matches!(event.command.as_str(), "about" | "solara" | "version") {
            event.response_lines.push(format!(
                "Solara-Sky v{} — Native Rust MC {} (protocol {}) with Folia-style multi-core region ticking.",
                env!("CARGO_PKG_VERSION"),
                sky_protocol::MINECRAFT_VERSION,
                sky_protocol::PROTOCOL_VERSION
            ));
            event.handled = true;
        }
    }
}

#[derive(Parser, Debug)]
#[command(
    name = "solara-sky",
    about = "High-performance Rust Minecraft: Java Edition 26.3 server"
)]
struct Cli {
    /// Directory containing server.properties, solara-sky.toml, ops.json, and world/region/
    #[arg(long, default_value = ".")]
    config_dir: PathBuf,

    /// Optional port override (overrides server-port in server.properties)
    #[arg(long)]
    port: Option<u16>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    std::fs::create_dir_all(&cli.config_dir)
        .with_context(|| format!("create config dir {}", cli.config_dir.display()))?;

    let mut props = ServerProperties::load_or_create(&cli.config_dir.join("server.properties"))?;
    if let Some(port) = cli.port {
        props.server_port = port;
    }
    let sky_cfg = SkyConfig::load_or_create(&cli.config_dir.join("solara-sky.toml"))?;

    info!(
        mc_version = sky_protocol::MINECRAFT_VERSION,
        protocol = sky_protocol::PROTOCOL_VERSION,
        online_mode = props.online_mode,
        proxy_mode = ?sky_cfg.proxy.mode,
        target_tps = sky_cfg.performance.target_tps,
        region_shift = sky_cfg.performance.region_shift_chunks,
        total_blocks = sky_world::generated_blocks::TOTAL_BLOCKS,
        total_block_states = sky_world::generated_blocks::TOTAL_BLOCK_STATES,
        "starting Solara-Sky server"
    );

    let bind_addr = format!("{}:{}", props.server_ip, props.server_port);
    let listener = TcpListener::bind(&bind_addr)
        .await
        .with_context(|| format!("bind TCP listener on {bind_addr}"))?;

    let mut plugins = PluginManager::new();
    let boot_info = ServerInfo {
        brand: "solara-sky".into(),
        minecraft_version: sky_protocol::MINECRAFT_VERSION.into(),
        protocol_version: sky_protocol::PROTOCOL_VERSION,
        online_players: 0,
        max_players: props.max_players,
    };
    plugins.register(Box::new(SolaraCorePlugin), &boot_info);

    let shared = Arc::new(SharedServerState::new(
        &cli.config_dir,
        props,
        sky_cfg,
        plugins,
    )?);
    let tick_handle = tick::spawn_region_tick_loop(Arc::clone(&shared));

    info!(address = %bind_addr, "Solara-Sky is listening for Minecraft 26.3 connections");

    loop {
        tokio::select! {
            accept_res = listener.accept() => {
                match accept_res {
                    Ok((stream, peer)) => {
                        let st = Arc::clone(&shared);
                        tokio::spawn(async move {
                            if let Err(err) = handle_client(stream, peer, st).await {
                                warn!(%peer, error = %err, "connection closed with protocol error");
                            }
                        });
                    }
                    Err(err) => warn!(error = %err, "TCP accept error"),
                }
            }
            _ = tokio::signal::ctrl_c() => {
                info!("received shutdown signal, stopping region ticker and saving modified Anvil chunks...");
                tick_handle.abort();
                match shared.world.save_dirty_chunks(false).await {
                    Ok(saved) => info!(saved_chunks = saved, "Anvil world save complete"),
                    Err(err) => warn!(error = %err, "failed to save dirty Anvil chunks on shutdown"),
                }
                shared.plugins.shutdown();
                break;
            }
        }
    }

    Ok(())
}
