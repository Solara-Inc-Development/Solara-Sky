pub mod config;
pub mod net;

use crate::config::{ServerProperties, SkyConfig};
use crate::net::{SharedServerState, handle_client};
use anyhow::{Context, Result};
use clap::Parser;
use sky_api::PluginManager;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::{info, warn};

#[derive(Parser, Debug)]
#[command(
    name = "solara-sky",
    about = "High-performance Rust Minecraft: Java Edition 26.3 server"
)]
struct Cli {
    /// Directory containing server.properties and solara-sky.toml
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
        total_blocks = sky_world::generated_blocks::TOTAL_BLOCKS,
        total_block_states = sky_world::generated_blocks::TOTAL_BLOCK_STATES,
        "starting Solara-Sky server"
    );

    let bind_addr = format!("{}:{}", props.server_ip, props.server_port);
    let listener = TcpListener::bind(&bind_addr)
        .await
        .with_context(|| format!("bind TCP listener on {bind_addr}"))?;

    let plugins = PluginManager::new();
    let shared = Arc::new(SharedServerState::new(props, sky_cfg, plugins)?);
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
                info!("received shutdown signal, stopping Solara-Sky cleanly");
                shared.plugins.shutdown();
                break;
            }
        }
    }

    Ok(())
}
