//! Plugin and event API foundations for Solara-Sky.
//!
//! Designed around zero-allocation-friendly hooks so Solara-Sky's native/WASM plugin ecosystem
//! can intercept server pings, authentication, chat, and world events without stalling ticks.

use uuid::Uuid;

/// Metadata describing a loaded Solara-Sky plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginMeta {
    pub id: String,
    pub name: String,
    pub version: String,
    pub authors: Vec<String>,
}

/// Snapshot of basic server runtime metadata exposed to plugins.
#[derive(Debug, Clone)]
pub struct ServerInfo {
    pub brand: String,
    pub minecraft_version: String,
    pub protocol_version: i32,
    pub online_players: usize,
    pub max_players: usize,
}

/// Mutable event fired when a client sends a Server List Ping.
#[derive(Debug, Clone)]
pub struct StatusPingEvent {
    pub description: String,
    pub online_players: usize,
    pub max_players: usize,
    pub protocol_version: i32,
}

/// Event fired after authentication/proxy verification succeeds, before entering Configuration.
#[derive(Debug, Clone)]
pub struct PlayerPreLoginEvent {
    pub username: String,
    pub uuid: Uuid,
    pub cancel_reason: Option<String>,
}

impl PlayerPreLoginEvent {
    pub fn cancel(&mut self, reason: impl Into<String>) {
        self.cancel_reason = Some(reason.into());
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel_reason.is_some()
    }
}

/// Core trait implemented by native Solara-Sky plugins.
pub trait SkyPlugin: Send + Sync {
    fn meta(&self) -> PluginMeta;

    fn on_enable(&self, _server: &ServerInfo) {}

    fn on_disable(&self) {}

    fn on_status_ping(&self, _event: &mut StatusPingEvent) {}

    fn on_pre_login(&self, _event: &mut PlayerPreLoginEvent) {}
}

/// Lightweight registry storing enabled plugins in load order.
#[derive(Default)]
pub struct PluginManager {
    plugins: Vec<Box<dyn SkyPlugin>>,
}

impl PluginManager {
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
        }
    }

    pub fn register(&mut self, plugin: Box<dyn SkyPlugin>, server: &ServerInfo) {
        plugin.on_enable(server);
        self.plugins.push(plugin);
    }

    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    pub fn fire_status_ping(&self, event: &mut StatusPingEvent) {
        for plugin in &self.plugins {
            plugin.on_status_ping(event);
        }
    }

    pub fn fire_pre_login(&self, event: &mut PlayerPreLoginEvent) {
        for plugin in &self.plugins {
            plugin.on_pre_login(event);
            if event.is_cancelled() {
                break;
            }
        }
    }

    pub fn shutdown(&self) {
        for plugin in self.plugins.iter().rev() {
            plugin.on_disable();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct CustomMotdPlugin;

    impl SkyPlugin for CustomMotdPlugin {
        fn meta(&self) -> PluginMeta {
            PluginMeta {
                id: "custom-motd".into(),
                name: "Custom MOTD".into(),
                version: "0.1.0".into(),
                authors: vec!["Solara".into()],
            }
        }

        fn on_status_ping(&self, event: &mut StatusPingEvent) {
            event.description = "Powered by Solara-Sky".into();
        }

        fn on_pre_login(&self, event: &mut PlayerPreLoginEvent) {
            if event.username == "BlockedUser" {
                event.cancel("Banned by plugin");
            }
        }
    }

    #[test]
    fn plugin_hooks_mutate_events() {
        let info = ServerInfo {
            brand: "solara-sky".into(),
            minecraft_version: "26.3".into(),
            protocol_version: 777,
            online_players: 0,
            max_players: 100,
        };
        let mut mgr = PluginManager::new();
        mgr.register(Box::new(CustomMotdPlugin), &info);
        assert_eq!(mgr.len(), 1);

        let mut ping = StatusPingEvent {
            description: "Default".into(),
            online_players: 0,
            max_players: 100,
            protocol_version: 777,
        };
        mgr.fire_status_ping(&mut ping);
        assert_eq!(ping.description, "Powered by Solara-Sky");

        let mut login = PlayerPreLoginEvent {
            username: "BlockedUser".into(),
            uuid: Uuid::nil(),
            cancel_reason: None,
        };
        mgr.fire_pre_login(&mut login);
        assert_eq!(login.cancel_reason.as_deref(), Some("Banned by plugin"));
    }
}
