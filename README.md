# Solara-Sky

A fast, lightweight, open-source **Minecraft: Java Edition** server written in native Rust, built for low latency, low memory usage, and zero garbage-collection stalls.

> **Status:** Active development (Phases 0–10 playable core complete across **13 modular crates**: MC `26.3` login/auth, Velocity/BungeeCord forwarding, Folia-style `rayon` multi-core regionized ticking, TPS/MSPT profiler, `sky-api` + `.lua` `sky-scripting` plugins, 5-biome Overworld terrain/structures/ores, dynamic Torch & Glowstone Sky/Block lighting, Chest & Furnace block entities, 8 Overworld mobs, 3D Bow/Arrow projectiles, Armor & Shield defense, XP levels, Status Potion Effects & Golden Apples, Beds & personal respawn points, 30 survival crafting recipes, interactive Doors/Ladders, weather & async Anvil `.mca` persistence).
> **Target Minecraft Version:** Java Edition **26.3** (*Wilderness Bound*, protocol **`777`**)
> **Supported Platforms:** Windows & Linux

## Goals

- **100% Standard Java Edition Protocol Compatibility:** Works with any standard Java client (Vanilla, Fabric, `Solara-Ground`, etc.).
- **Online & Offline Mode:** Toggleable via standard `server.properties` (`online-mode=true/false`), with full RSA-1024 + AES-128-CFB8 encryption and Mojang session verification in online mode.
- **First-Class Proxy Support:** Native **Velocity modern forwarding** (HMAC-SHA256) and **BungeeCord** IP forwarding via `solara-sky.toml`.
- **Fast Version Updates:** Built-in `tools/mc-codegen` imports official Mojang data reports (`--reports`) for packet IDs, block states, and registries.
- **Extensible Architecture:** Clean `sky-api` crate for Solara-Sky's own high-performance plugin API.

## Workspace Layout

- `crates/sky-data` — Generated MC `26.3` (`777`) items (`1,658`), entity types (`161`), sound events (`1,991`), particles (`128`), and mob effects (`40`)
- `crates/sky-protocol` — MC `26.3` (`777`) protocol primitives (`VarInt`, NBT, compression, AES-128-CFB8 crypto, packets, survival/combat codecs, Velocity/BungeeCord forwarding)
- `crates/sky-world` — Bit-packed paletted chunk storage, `rayon` multi-core terrain/cave/tree generator, 24-section Sky/Block lighting, and async Anvil (`.mca`) persistence
- `crates/sky-inventory` — `ItemStack`, 36-slot `PlayerInventory` + 4 Armor slots, and 30 core crafting recipes
- `crates/sky-block-entity` — Persistent 27-slot `ChestBlockEntity` storage and Folia-ticked `FurnaceBlockEntity` ore/food smelting (`raw_iron`, `raw_gold`, `sand`, `cobblestone`, `beef`, `porkchop`)
- `crates/sky-effects` — Server-ticked Status Potion Effects (`Speed`, `Strength`, `Regeneration`, `Resistance`, `Night Vision`) and Golden Apple buffs
- `crates/sky-entity` — Survival `PlayerVitals` (health, hunger, armor mitigation, XP levels, fall damage, regen), melee knockback, floating `WorldItemDrop` loot, and 8-species `WorldMob` AI
- `crates/sky-projectile` — Ballistic 3D `WorldArrow` projectiles (`minecraft:arrow`) with swept AABB mob/PvP hit detection and shooter XP rewards
- `crates/sky-command` — Brigadier-style command catalog and `Tab` auto-completion engine
- `crates/sky-api` — Native Rust server plugin & event SDK (`SkyPlugin`)
- `crates/sky-scripting` — Lightweight `.lua` plugin loader (`plugins/*.lua`) for custom commands, MOTD overrides, and chat filters without recompiling Rust
- `crates/sky-server` — Standalone `solara-sky` server binary, Folia-style parallel region ticker, TPS/MSPT profiler, and `ops.json`/`whitelist.json`/`banned-players.json` ACL
- `docs/` & `packaging/` — Architecture guides, tuning docs, Docker & systemd deployment configs
- `tools/mc-codegen` — Code generator from Mojang official `--reports` JSON

## Building & Testing

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run --bin solara-sky
```

## License & Legal Notice

Licensed under the **GNU General Public License v3.0** — see [LICENSE](./LICENSE).

`Solara-Sky` is an independent, clean-room Rust implementation of the Minecraft: Java Edition server protocol and contains zero Mojang code or redistributed game assets.

> **NOT AN OFFICIAL MINECRAFT PRODUCT. NOT APPROVED BY OR ASSOCIATED WITH MOJANG OR MICROSOFT.**
