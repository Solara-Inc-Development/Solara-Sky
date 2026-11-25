//! Vanilla-compatible `ops.json`, `whitelist.json`, and `banned-players.json` access control.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpEntry {
    pub uuid: Uuid,
    pub name: String,
    pub level: u8,
    #[serde(rename = "bypassesPlayerLimit")]
    pub bypasses_player_limit: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhitelistEntry {
    pub uuid: Uuid,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BanEntry {
    pub uuid: Uuid,
    pub name: String,
    pub reason: String,
}

/// Thread-safe operator, whitelist, and player ban store backed by JSON files in `config_dir`.
pub struct AccessControl {
    ops_path: PathBuf,
    whitelist_path: PathBuf,
    bans_path: PathBuf,
    ops: RwLock<BTreeMap<String, OpEntry>>,
    whitelist: RwLock<BTreeMap<String, WhitelistEntry>>,
    bans: RwLock<BTreeMap<String, BanEntry>>,
}

fn load_json_vec<T: for<'de> Deserialize<'de> + Serialize>(path: &Path) -> Result<Vec<T>> {
    if !path.exists() {
        fs::write(path, "[]\n").with_context(|| format!("create {}", path.display()))?;
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))
}

fn save_json_vec<T: Serialize>(path: &Path, items: &[T]) -> Result<()> {
    let raw = serde_json::to_string_pretty(items).context("serialize access control JSON")?;
    fs::write(path, format!("{raw}\n")).with_context(|| format!("write {}", path.display()))
}

impl AccessControl {
    pub fn load_or_create(config_dir: &Path) -> Result<Self> {
        let ops_path = config_dir.join("ops.json");
        let whitelist_path = config_dir.join("whitelist.json");
        let bans_path = config_dir.join("banned-players.json");

        let ops_list: Vec<OpEntry> = load_json_vec(&ops_path)?;
        let wl_list: Vec<WhitelistEntry> = load_json_vec(&whitelist_path)?;
        let ban_list: Vec<BanEntry> = load_json_vec(&bans_path)?;

        let ops = ops_list
            .into_iter()
            .map(|e| (e.name.to_ascii_lowercase(), e))
            .collect();
        let whitelist = wl_list
            .into_iter()
            .map(|e| (e.name.to_ascii_lowercase(), e))
            .collect();
        let bans = ban_list
            .into_iter()
            .map(|e| (e.name.to_ascii_lowercase(), e))
            .collect();

        Ok(Self {
            ops_path,
            whitelist_path,
            bans_path,
            ops: RwLock::new(ops),
            whitelist: RwLock::new(whitelist),
            bans: RwLock::new(bans),
        })
    }

    /// Returns `true` if `username` is an operator (or when `ops.json` is empty on a fresh dev server
    /// only if explicitly granted; console/explicit ops have `level >= 4`).
    pub fn is_op(&self, username: &str) -> bool {
        self.ops
            .read()
            .expect("ops lock")
            .contains_key(&username.to_ascii_lowercase())
    }

    pub fn has_any_ops(&self) -> bool {
        !self.ops.read().expect("ops lock").is_empty()
    }

    pub fn ban_reason(&self, username: &str, uuid: Uuid) -> Option<String> {
        let guard = self.bans.read().expect("bans lock");
        if let Some(b) = guard.get(&username.to_ascii_lowercase()) {
            return Some(b.reason.clone());
        }
        guard
            .values()
            .find(|b| b.uuid == uuid)
            .map(|b| b.reason.clone())
    }

    pub fn is_whitelisted(&self, username: &str, uuid: Uuid) -> bool {
        if self.is_op(username) {
            return true;
        }
        let guard = self.whitelist.read().expect("whitelist lock");
        guard.contains_key(&username.to_ascii_lowercase()) || guard.values().any(|w| w.uuid == uuid)
    }

    pub fn grant_op(&self, username: &str, uuid: Uuid) -> Result<()> {
        let list: Vec<OpEntry> = {
            let mut guard = self.ops.write().expect("ops lock");
            guard.insert(
                username.to_ascii_lowercase(),
                OpEntry {
                    uuid,
                    name: username.to_owned(),
                    level: 4,
                    bypasses_player_limit: true,
                },
            );
            guard.values().cloned().collect()
        };
        save_json_vec(&self.ops_path, &list)
    }

    pub fn revoke_op(&self, username: &str) -> Result<bool> {
        let (removed, list): (bool, Vec<OpEntry>) = {
            let mut guard = self.ops.write().expect("ops lock");
            let existed = guard.remove(&username.to_ascii_lowercase()).is_some();
            (existed, guard.values().cloned().collect())
        };
        if removed {
            save_json_vec(&self.ops_path, &list)?;
        }
        Ok(removed)
    }

    pub fn add_whitelist(&self, username: &str, uuid: Uuid) -> Result<()> {
        let list: Vec<WhitelistEntry> = {
            let mut guard = self.whitelist.write().expect("whitelist lock");
            guard.insert(
                username.to_ascii_lowercase(),
                WhitelistEntry {
                    uuid,
                    name: username.to_owned(),
                },
            );
            guard.values().cloned().collect()
        };
        save_json_vec(&self.whitelist_path, &list)
    }

    pub fn remove_whitelist(&self, username: &str) -> Result<bool> {
        let (removed, list): (bool, Vec<WhitelistEntry>) = {
            let mut guard = self.whitelist.write().expect("whitelist lock");
            let existed = guard.remove(&username.to_ascii_lowercase()).is_some();
            (existed, guard.values().cloned().collect())
        };
        if removed {
            save_json_vec(&self.whitelist_path, &list)?;
        }
        Ok(removed)
    }

    pub fn ban_player(&self, username: &str, uuid: Uuid, reason: &str) -> Result<()> {
        let list: Vec<BanEntry> = {
            let mut guard = self.bans.write().expect("bans lock");
            guard.insert(
                username.to_ascii_lowercase(),
                BanEntry {
                    uuid,
                    name: username.to_owned(),
                    reason: reason.to_owned(),
                },
            );
            guard.values().cloned().collect()
        };
        save_json_vec(&self.bans_path, &list)
    }

    pub fn pardon_player(&self, username: &str) -> Result<bool> {
        let (removed, list): (bool, Vec<BanEntry>) = {
            let mut guard = self.bans.write().expect("bans lock");
            let existed = guard.remove(&username.to_ascii_lowercase()).is_some();
            (existed, guard.values().cloned().collect())
        };
        if removed {
            save_json_vec(&self.bans_path, &list)?;
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ops_whitelist_and_bans_persist_and_roundtrip() {
        let tmp = std::env::temp_dir().join(format!("solara-sky-acl-{}", Uuid::new_v4()));
        fs::create_dir_all(&tmp).expect("mkdir");
        let uid = Uuid::new_v4();

        let acl = AccessControl::load_or_create(&tmp).expect("init acl");
        acl.grant_op("SolaraAdmin", uid).expect("op");
        acl.add_whitelist("BuilderOne", Uuid::new_v4()).expect("wl");
        acl.ban_player("Griefer99", Uuid::new_v4(), "Griefing spawn")
            .expect("ban");

        let reloaded = AccessControl::load_or_create(&tmp).expect("reload acl");
        assert!(reloaded.is_op("solaraadmin"));
        assert!(reloaded.is_whitelisted("builderone", Uuid::nil()));
        assert_eq!(
            reloaded.ban_reason("griefer99", Uuid::nil()).as_deref(),
            Some("Griefing spawn")
        );
        assert!(reloaded.pardon_player("Griefer99").expect("pardon"));
        assert!(reloaded.ban_reason("Griefer99", Uuid::nil()).is_none());

        let _ = fs::remove_dir_all(&tmp);
    }
}
