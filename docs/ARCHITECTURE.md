# Solara-Sky Architecture Overview

**Solara-Sky** is a clean-room, multi-core native Rust server implementation for **Minecraft: Java Edition 26.3** (protocol **`777`**).

## Workspace Crate Map

| Crate | Responsibility |
| :--- | :--- |
| `crates/sky-data` | Generated protocol registry tables (`1,658` items, `161` entity types, `1,991` sound events, `128` particles, `40` mob effects) |
| `crates/sky-protocol` | Zero-panic wire framing, zlib compression, RSA + AES-128-CFB8 encryption, Network/Disk NBT, Velocity HMAC-SHA256 & BungeeCord forwarding, Play/Survival codecs |
| `crates/sky-world` | Bit-packed paletted chunks, `rayon` multi-core Overworld terrain/cave/tree generator, 24-section BFS Sky & Block lighting, and async Anvil (`.mca`) persistence |
| `crates/sky-inventory` | `ItemStack`, 36-slot `PlayerInventory`, armor/offhand slots, and core crafting recipes |
| `crates/sky-entity` | `PlayerVitals` (20 HP, 20 hunger, fall damage, hurt immunity, natural regen), PvP/PvE knockback math, and `WorldMob` AI |
| `crates/sky-command` | Brigadier-style command metadata and `Tab` auto-completion engine |
| `crates/sky-api` | Native high-performance plugin trait (`SkyPlugin`) and event hooks |
| `crates/sky-server` | Async `tokio` TCP server, Folia-style parallel region ticker, TPS/MSPT profiler, and `ops.json`/`whitelist.json`/`banned-players.json` ACL |
| `tools/mc-codegen` | Data-report code generator importing official `--reports` JSON |

## Concurrency & Performance Model

1. **Async Network Workers (`tokio`):** Handle TCP sockets, AES-128-CFB8 stream encryption, zlib framing, and per-client packet rate limiting without blocking world ticks.
2. **Multi-Core Worldgen & Lighting (`rayon`):** Missing chunks are generated, lit across 24 vertical sections, and serialized into zero-copy `Bytes` packets across all CPU cores simultaneously.
3. **Folia-Style Parallel Region Ticking:** Active simulation chunks are partitioned into `16x16` chunk spatial regions and ticked concurrently on `rayon` worker threads at `20 TPS`.

> NOT AN OFFICIAL MINECRAFT PRODUCT. NOT APPROVED BY OR ASSOCIATED WITH MOJANG OR MICROSOFT.
