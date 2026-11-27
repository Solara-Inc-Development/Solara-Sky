//! Lightweight `.lua` script plugin loader and interpreter for Solara-Sky.
//!
//! Allows server administrators to drop `.lua` plugin scripts into `plugins/*.lua` to add custom
//! commands, MOTD overrides, and chat filters/macros without recompiling Rust, while keeping
//! zero C toolchain dependencies across Windows and Linux builds.

use sky_api::{ChatEvent, CommandEvent, PluginMeta, SkyPlugin, StatusPingEvent};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use thiserror::Error;
use tracing::info;

#[derive(Debug, Error)]
pub enum ScriptError {
    #[error("I/O error loading Lua plugin: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid Lua plugin syntax at line {line}: {message}")]
    Syntax { line: usize, message: String },
}

/// Declarative + rule-driven Lua plugin compiled into fast in-memory hooks.
#[derive(Debug, Clone)]
pub struct LuaScriptPlugin {
    meta: PluginMeta,
    motd_override: Option<String>,
    custom_commands: HashMap<String, Vec<String>>,
    blocked_words: Vec<String>,
}

impl LuaScriptPlugin {
    /// Parses a Solara-Sky `.lua` script source string.
    ///
    /// Supported clean Lua constructs:
    /// - `PLUGIN = { id = "...", name = "...", version = "...", author = "..." }` (or top-level `plugin_id = "..."`)
    /// - `motd = "..."`
    /// - `command("rules", "1. Be respectful\\n2. Have fun!")`
    /// - `block_word("spamword")`
    pub fn from_lua_source(fallback_id: &str, src: &str) -> Result<Self, ScriptError> {
        let mut id = fallback_id.to_owned();
        let mut name = fallback_id.to_owned();
        let mut version = "0.1.0".to_owned();
        let mut author = "ServerAdmin".to_owned();
        let mut motd_override = None;
        let mut custom_commands: HashMap<String, Vec<String>> = HashMap::new();
        let mut blocked_words = Vec::new();

        for (idx, raw_line) in src.lines().enumerate() {
            let line_no = idx + 1;
            let trimmed = strip_lua_comment(raw_line).trim();
            if trimmed.is_empty()
                || trimmed.starts_with("local ") && trimmed.ends_with('{')
                || trimmed == "}"
                || trimmed.starts_with("function ")
                || trimmed == "end"
            {
                continue;
            }

            if let Some(args) = parse_call(trimmed, "command") {
                let (cmd_name, reply) = split_two_string_args(args, line_no)?;
                let lines = reply
                    .split("\\n")
                    .map(|s| s.trim().to_owned())
                    .filter(|s| !s.is_empty())
                    .collect();
                custom_commands
                    .insert(cmd_name.trim_start_matches('/').to_ascii_lowercase(), lines);
                continue;
            }

            if let Some(args) = parse_call(trimmed, "block_word") {
                let word = unquote_lua_string(args.trim(), line_no)?;
                if !word.is_empty() {
                    blocked_words.push(word.to_ascii_lowercase());
                }
                continue;
            }

            if let Some((k, v)) = trimmed.trim_end_matches(',').split_once('=') {
                let key = k.trim().trim_start_matches("local ").trim();
                let val_raw = v.trim();
                if val_raw.starts_with('"') || val_raw.starts_with('\'') {
                    let val = unquote_lua_string(val_raw, line_no)?;
                    match key {
                        "plugin_id" | "id" => id = val,
                        "plugin_name" | "name" => name = val,
                        "plugin_version" | "version" => version = val,
                        "plugin_author" | "author" => author = val,
                        "motd" => motd_override = Some(val),
                        _ => {}
                    }
                }
            }
        }

        Ok(Self {
            meta: PluginMeta {
                id,
                name,
                version,
                authors: vec![author],
            },
            motd_override,
            custom_commands,
            blocked_words,
        })
    }

    /// Loads all `*.lua` scripts from `dir` (creating a default `welcome.lua` if the folder is new).
    pub fn load_directory(dir: &Path) -> Result<Vec<Self>, ScriptError> {
        if !dir.exists() {
            fs::create_dir_all(dir)?;
            let sample = dir.join("welcome.lua");
            fs::write(
                &sample,
                concat!(
                    "-- Solara-Sky Lua Plugin Script (auto-loaded at startup)\n",
                    "plugin_id = \"solara-welcome-lua\"\n",
                    "plugin_name = \"WelcomeLua\"\n",
                    "plugin_version = \"1.0.0\"\n",
                    "plugin_author = \"Solara-Sky\"\n\n",
                    "command(\"rules\", \"Solara-Sky Rules:\\n1. Respect other players\\n2. Mine ores, craft gear & enjoy 20 TPS!\")\n",
                    "command(\"guide\", \"Quick Guide: Press E for inventory/crafting, T to chat, F to toggle walk/fly, R to respawn!\")\n",
                ),
            )?;
        }

        let mut loaded = Vec::new();
        let mut paths = Vec::new();
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("lua") {
                paths.push(path);
            }
        }
        paths.sort();

        for path in paths {
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("lua-script");
            let raw = fs::read_to_string(&path)?;
            let plugin = Self::from_lua_source(stem, &raw)?;
            info!(
                plugin = %plugin.meta.name,
                commands = plugin.custom_commands.len(),
                path = %path.display(),
                "loaded Lua script plugin"
            );
            loaded.push(plugin);
        }
        Ok(loaded)
    }
}

impl SkyPlugin for LuaScriptPlugin {
    fn meta(&self) -> PluginMeta {
        self.meta.clone()
    }

    fn on_status_ping(&self, event: &mut StatusPingEvent) {
        if let Some(motd) = &self.motd_override {
            event.description.clone_from(motd);
        }
    }

    fn on_chat(&self, event: &mut ChatEvent) {
        if self.blocked_words.is_empty() {
            return;
        }
        let lower = event.message.to_ascii_lowercase();
        if self.blocked_words.iter().any(|w| lower.contains(w)) {
            event.cancelled = true;
        }
    }

    fn on_command(&self, event: &mut CommandEvent) {
        if let Some(lines) = self.custom_commands.get(&event.command) {
            for line in lines {
                event
                    .response_lines
                    .push(line.replace("{player}", &event.sender_username));
            }
            event.handled = true;
        }
    }
}

fn strip_lua_comment(line: &str) -> &str {
    let mut in_quote: Option<char> = None;
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let ch = bytes[i] as char;
        if let Some(q) = in_quote {
            if ch == '\\' {
                i += 2;
                continue;
            }
            if ch == q {
                in_quote = None;
            }
        } else if ch == '"' || ch == '\'' {
            in_quote = Some(ch);
        } else if ch == '-' && i + 1 < bytes.len() && bytes[i + 1] == b'-' {
            return &line[..i];
        }
        i += 1;
    }
    line
}

fn parse_call<'a>(line: &'a str, fn_name: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(fn_name)?.trim_start();
    let inner = rest.strip_prefix('(')?.strip_suffix(')')?;
    Some(inner.trim())
}

fn unquote_lua_string(raw: &str, line: usize) -> Result<String, ScriptError> {
    let bytes = raw.as_bytes();
    if bytes.len() < 2 {
        return Err(ScriptError::Syntax {
            line,
            message: "expected quoted Lua string".into(),
        });
    }
    let quote = bytes[0] as char;
    if (quote != '"' && quote != '\'') || bytes[bytes.len() - 1] as char != quote {
        return Err(ScriptError::Syntax {
            line,
            message: "mismatched Lua string quotes".into(),
        });
    }
    Ok(raw[1..raw.len() - 1].replace("\\\"", "\""))
}

fn split_two_string_args(args: &str, line: usize) -> Result<(String, String), ScriptError> {
    let mut in_quote: Option<char> = None;
    let bytes = args.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let ch = bytes[i] as char;
        if let Some(q) = in_quote {
            if ch == '\\' {
                i += 2;
                continue;
            }
            if ch == q {
                in_quote = None;
            }
        } else if ch == '"' || ch == '\'' {
            in_quote = Some(ch);
        } else if ch == ',' {
            let left = unquote_lua_string(args[..i].trim(), line)?;
            let right = unquote_lua_string(args[i + 1..].trim(), line)?;
            return Ok((left, right));
        }
        i += 1;
    }
    Err(ScriptError::Syntax {
        line,
        message: "expected two comma-separated string arguments".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn lua_plugin_handles_custom_commands_and_chat_filters() {
        let src = r#"
            -- Sample server Lua plugin
            plugin_id = "test-lua"
            plugin_name = "TestLua"
            plugin_version = "1.2.0"
            command("hello", "Welcome {player}!\nEnjoy Solara-Sky!")
            block_word("badword")
        "#;
        let plugin = LuaScriptPlugin::from_lua_source("fallback", src).expect("parse lua");
        assert_eq!(plugin.meta().name, "TestLua");

        let mut cmd = CommandEvent {
            sender_username: "Alex".into(),
            sender_uuid: Uuid::nil(),
            is_op: false,
            command: "hello".into(),
            args: vec![],
            response_lines: vec![],
            handled: false,
        };
        plugin.on_command(&mut cmd);
        assert!(cmd.handled);
        assert_eq!(cmd.response_lines[0], "Welcome Alex!");

        let mut chat = ChatEvent {
            sender_username: "Alex".into(),
            sender_uuid: Uuid::nil(),
            message: "this contains BADWORD inside".into(),
            cancelled: false,
        };
        plugin.on_chat(&mut chat);
        assert!(chat.cancelled);
    }
}
