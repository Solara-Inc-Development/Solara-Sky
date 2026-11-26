# Solara-Sky Plugin SDK (`sky-api`)

`sky-api` provides zero-allocation-friendly synchronous event hooks executed cleanly inside `Solara-Sky`:

- `on_status_ping(&mut StatusPingEvent)` — Customize MOTD, player counts, and protocol advertisement.
- `on_pre_login(&mut PlayerPreLoginEvent)` — Custom authentication, maintenance mode, or ban gates.
- `on_chat(&mut ChatEvent)` — Chat formatting, profanity filters, and channel routing.
- `on_block_change(&mut BlockChangeEvent)` — Spawn protection, region claims, and grief prevention.
- `on_command(&mut CommandEvent)` — Custom `/command` handlers with operator permission context.
- `on_tick(&RegionTickSummary)` — Per-tick MSPT, TPS, and active Folia region telemetry.

> NOT AN OFFICIAL MINECRAFT PRODUCT. NOT APPROVED BY OR ASSOCIATED WITH MOJANG OR MICROSOFT.
