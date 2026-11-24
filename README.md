# Solara-Sky

A fast, lightweight, open-source **Minecraft: Java Edition** server written in native Rust, built for low latency, low memory usage, and zero garbage-collection stalls.

> **Status:** Early active development (Phase 0 / Phase 1 networking foundation). Not yet a complete survival server.
> **Target Minecraft Version:** Java Edition **26.3** (*Wilderness Bound*, protocol **`777`**)
> **Supported Platforms:** Windows & Linux

## Goals

- **100% Standard Java Edition Protocol Compatibility:** Works with any standard Java client (Vanilla, Fabric, `Solara-Ground`, etc.).
- **Online & Offline Mode:** Toggleable via standard `server.properties` (`online-mode=true/false`), with full RSA-1024 + AES-128-CFB8 encryption and Mojang session verification in online mode.
- **First-Class Proxy Support:** Native **Velocity modern forwarding** (HMAC-SHA256) and **BungeeCord** IP forwarding via `solara-sky.toml`.
- **Fast Version Updates:** Built-in `tools/mc-codegen` imports official Mojang data reports (`--reports`) for packet IDs, block states, and registries.
- **Extensible Architecture:** Clean `sky-api` crate for Solara-Sky's own high-performance plugin API.

## Workspace Layout

- `crates/sky-protocol` — MC `26.3` (`777`) protocol primitives (`VarInt`, NBT, compression, AES-128-CFB8 crypto, packets, Velocity/BungeeCord forwarding)
- `crates/sky-world` — Bit-packed paletted chunk storage, coordinates, generated block state metadata
- `crates/sky-api` — Server plugin & event API foundations
- `crates/sky-server` — Standalone `solara-sky` async TCP server binary (`server.properties` + `solara-sky.toml`)
- `tools/mc-codegen` — Code generator from Mojang official `--reports` JSON

## Building & Testing

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run --bin solara-sky
```

## License

Licensed under the **GNU General Public License v3.0** — see [LICENSE](./LICENSE).
