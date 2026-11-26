# Solara-Sky Performance & Proxy Tuning Guide

## 1. `server.properties`
- `online-mode=true` — Enables RSA-1024 + AES-128-CFB8 + Mojang `hasJoined` verification (`false` enables standard offline UUIDv3).
- `view-distance=10` / `simulation-distance=6` — Recommended balance for 100+ players on modest CPUs.
- `network-compression-threshold=256` — Compresses chunk & registry frames while keeping small movement packets raw.

## 2. `solara-sky.toml`
```toml
[proxy]
mode = "none" # "none" | "velocity" | "bungeecord"
velocity_secret = ""

[performance]
max_packet_bytes = 2097152
target_tps = 20
region_shift_chunks = 4 # 16x16 chunks per parallel Folia region
random_tick_speed = 3
autosave_interval_ticks = 6000
packet_rate_limit_per_sec = 600
```

> NOT AN OFFICIAL MINECRAFT PRODUCT. NOT APPROVED BY OR ASSOCIATED WITH MOJANG OR MICROSOFT.
