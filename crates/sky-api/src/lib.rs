//! Plugin and event API for Solara-Sky.
//!
//! Designed around zero-allocation-friendly hooks so Solara-Sky's native Rust (and future WASM)
//! plugin ecosystem can intercept server pings, authentication, chat, block mutations, custom
//! commands, and Folia-style region tick summaries without stalling server ticks.

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

/// Mutable chat event fired before broadcasting a player chat message.
#[derive(Debug, Clone)]
pub struct ChatEvent {
    pub sender_username: String,
    pub sender_uuid: Uuid,
    pub message: String,
    pub cancelled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockChangeKind {
    Break,
    Place,
}

/// Mutable world protection / audit event fired before breaking or placing a block.
#[derive(Debug, Clone)]
pub struct BlockChangeEvent {
    pub player_username: String,
    pub player_uuid: Uuid,
    pub pos: (i32, i32, i32),
    pub kind: BlockChangeKind,
    pub new_state_id: u32,
    pub cancelled: bool,
}

/// Mutable custom command hook fired before built-in command fallback.
#[derive(Debug, Clone)]
pub struct CommandEvent {
    pub sender_username: String,
    pub sender_uuid: Uuid,
    pub is_op: bool,
    pub command: String,
    pub args: Vec<String>,
    pub response_lines: Vec<String>,
    pub handled: bool,
}

/// Telemetry snapshot delivered to plugins at the end of each parallel region tick.
#[derive(Debug, Clone, Copy)]
pub struct RegionTickSummary {
    pub tick_number: u64,
    pub active_regions: usize,
    pub random_updates_applied: usize,
    pub last_mspt: f64,
    pub tps_5s: f64,
}

/// Core trait implemented by native Solara-Sky plugins.
pub trait SkyPlugin: Send + Sync {
    fn meta(&self) -> PluginMeta;

    fn on_enable(&self, _server: &ServerInfo) {}

    fn on_disable(&self) {}

    fn on_status_ping(&self, _event: &mut StatusPingEvent) {}

    fn on_pre_login(&self, _event: &mut PlayerPreLoginEvent) {}

    fn on_chat(&self, _event: &mut ChatEvent) {}

    fn on_block_change(&self, _event: &mut BlockChangeEvent) {}

    fn on_command(&self, _event: &mut CommandEvent) {}

    fn on_tick(&self, _summary: &RegionTickSummary) {}
}

/// Thread-safe registry storing enabled plugins in deterministic load order.
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

    pub fn plugin_metas(&self) -> Vec<PluginMeta> {
        self.plugins.iter().map(|p| p.meta()).collect()
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

    pub fn fire_chat(&self, event: &mut ChatEvent) {
        for plugin in &self.plugins {
            plugin.on_chat(event);
            if event.cancelled {
                break;
            }
        }
    }

    pub fn fire_block_change(&self, event: &mut BlockChangeEvent) {
        for plugin in &self.plugins {
            plugin.on_block_change(event);
            if event.cancelled {
                break;
            }
        }
    }

    pub fn fire_command(&self, event: &mut CommandEvent) {
        for plugin in &self.plugins {
            plugin.on_command(event);
            if event.handled {
                break;
            }
        }
    }

    pub fn fire_tick(&self, summary: &RegionTickSummary) {
        for plugin in &self.plugins {
            plugin.on_tick(summary);
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

    struct GuardAndCommandPlugin;

    impl SkyPlugin for GuardAndCommandPlugin {
        fn meta(&self) -> PluginMeta {
            PluginMeta {
                id: "solara-guard".into(),
                name: "SolaraGuard".into(),
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

        fn on_block_change(&self, event: &mut BlockChangeEvent) {
            if event.pos == (8, -59, 8) && event.kind == BlockChangeKind::Break {
                event.cancelled = true;
            }
        }

        fn on_command(&self, event: &mut CommandEvent) {
            if event.command == "pingplugin" {
                event.response_lines.push("Pong from SolaraGuard!".into());
                event.handled = true;
            }
        }
    }

    #[test]
    fn plugin_hooks_mutate_and_guard_events() {
        let info = ServerInfo {
            brand: "solara-sky".into(),
            minecraft_version: "26.3".into(),
            protocol_version: 777,
            online_players: 0,
            max_players: 100,
        };
        let mut mgr = PluginManager::new();
        mgr.register(Box::new(GuardAndCommandPlugin), &info);
        assert_eq!(mgr.len(), 1);
        assert_eq!(mgr.plugin_metas()[0].name, "SolaraGuard");

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

        let mut brk = BlockChangeEvent {
            player_username: "Griefer".into(),
            player_uuid: Uuid::nil(),
            pos: (8, -59, 8),
            kind: BlockChangeKind::Break,
            new_state_id: 0,
            cancelled: false,
        };
        mgr.fire_block_change(&mut brk);
        assert!(brk.cancelled);

        let mut cmd = CommandEvent {
            sender_username: "Admin".into(),
            sender_uuid: Uuid::nil(),
            is_op: true,
            command: "pingplugin".into(),
            args: vec![],
            response_lines: vec![],
            handled: false,
        };
        mgr.fire_command(&mut cmd);
        assert!(cmd.handled);
        assert_eq!(cmd.response_lines, vec!["Pong from SolaraGuard!"]);
    }
}
