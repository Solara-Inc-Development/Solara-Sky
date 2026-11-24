use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// Standard `server.properties` configuration compatible with Vanilla & Paper servers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerProperties {
    pub server_ip: String,
    pub server_port: u16,
    pub motd: String,
    pub max_players: usize,
    pub online_mode: bool,
    pub view_distance: u8,
    pub simulation_distance: u8,
    pub network_compression_threshold: i32,
    pub gamemode: String,
    pub difficulty: String,
}

impl Default for ServerProperties {
    fn default() -> Self {
        Self {
            server_ip: "0.0.0.0".into(),
            server_port: 25565,
            motd: "A Solara-Sky Rust Server (MC 26.3)".into(),
            max_players: 100,
            online_mode: true,
            view_distance: 10,
            simulation_distance: 10,
            network_compression_threshold: 256,
            gamemode: "survival".into(),
            difficulty: "normal".into(),
        }
    }
}

impl ServerProperties {
    pub fn load_or_create(path: &Path) -> Result<Self> {
        if !path.exists() {
            let props = Self::default();
            fs::write(path, props.to_properties_string())
                .with_context(|| format!("write default {}", path.display()))?;
            return Ok(props);
        }
        let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        Ok(Self::parse(&raw))
    }

    pub fn parse(raw: &str) -> Self {
        let mut map = BTreeMap::new();
        for line in raw.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = trimmed.split_once('=') {
                map.insert(k.trim(), v.trim());
            }
        }

        let mut p = Self::default();
        if let Some(&v) = map.get("server-ip")
            && !v.is_empty()
        {
            p.server_ip = v.to_owned();
        }
        if let Some(v) = map.get("server-port").and_then(|s| s.parse().ok()) {
            p.server_port = v;
        }
        if let Some(&v) = map.get("motd") {
            p.motd = v.to_owned();
        }
        if let Some(v) = map.get("max-players").and_then(|s| s.parse().ok()) {
            p.max_players = v;
        }
        if let Some(v) = map.get("online-mode").and_then(|s| s.parse().ok()) {
            p.online_mode = v;
        }
        if let Some(v) = map.get("view-distance").and_then(|s| s.parse().ok()) {
            p.view_distance = v;
        }
        if let Some(v) = map.get("simulation-distance").and_then(|s| s.parse().ok()) {
            p.simulation_distance = v;
        }
        if let Some(v) = map
            .get("network-compression-threshold")
            .and_then(|s| s.parse().ok())
        {
            p.network_compression_threshold = v;
        }
        if let Some(&v) = map.get("gamemode") {
            p.gamemode = v.to_owned();
        }
        if let Some(&v) = map.get("difficulty") {
            p.difficulty = v.to_owned();
        }
        p
    }

    pub fn to_properties_string(&self) -> String {
        format!(
            "# Solara-Sky server properties (Minecraft 26.3)\n\
             server-ip={}\n\
             server-port={}\n\
             motd={}\n\
             max-players={}\n\
             online-mode={}\n\
             view-distance={}\n\
             simulation-distance={}\n\
             network-compression-threshold={}\n\
             gamemode={}\n\
             difficulty={}\n",
            self.server_ip,
            self.server_port,
            self.motd,
            self.max_players,
            self.online_mode,
            self.view_distance,
            self.simulation_distance,
            self.network_compression_threshold,
            self.gamemode,
            self.difficulty
        )
    }
}

/// Proxy forwarding mode configured in `solara-sky.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ProxyMode {
    #[default]
    None,
    Velocity,
    Bungeecord,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyConfig {
    pub mode: ProxyMode,
    pub velocity_secret: String,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            mode: ProxyMode::None,
            velocity_secret: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceConfig {
    pub max_packet_bytes: usize,
}

impl Default for PerformanceConfig {
    fn default() -> Self {
        Self {
            max_packet_bytes: 2 * 1024 * 1024,
        }
    }
}

/// Advanced Solara-Sky configuration (`solara-sky.toml`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkyConfig {
    #[serde(default)]
    pub proxy: ProxyConfig,
    #[serde(default)]
    pub performance: PerformanceConfig,
}

impl SkyConfig {
    pub fn load_or_create(path: &Path) -> Result<Self> {
        if !path.exists() {
            let cfg = Self::default();
            let raw = toml::to_string_pretty(&cfg).context("serialize default solara-sky.toml")?;
            fs::write(path, raw).with_context(|| format!("write {}", path.display()))?;
            return Ok(cfg);
        }
        let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        toml::from_str(&raw).with_context(|| format!("parse {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_online_and_offline_server_properties() {
        let raw = "online-mode=false\nserver-port=25570\nmotd=Cracked Test\n";
        let props = ServerProperties::parse(raw);
        assert!(!props.online_mode);
        assert_eq!(props.server_port, 25570);
        assert_eq!(props.motd, "Cracked Test");
    }
}
