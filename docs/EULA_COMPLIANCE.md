# Clean-Room & Minecraft EULA Compliance

`Solara-Sky` is a 100% clean-room native Rust server implementation licensed under **GPL-3.0-only**:

- **Zero Mojang Assets or Bytecode:** No `.jar`, `.png`, `.ogg`, models, or Mojang source/bytecode are stored or redistributed in this repository.
- **Data-Driven Protocol Interoperability:** Numeric protocol IDs (`packets`, `blocks`, `items`, `entities`, `sounds`) are generated into Rust types via `tools/mc-codegen` strictly for wire compatibility with standard Minecraft: Java Edition 26.3 clients.

> NOT AN OFFICIAL MINECRAFT PRODUCT. NOT APPROVED BY OR ASSOCIATED WITH MOJANG OR MICROSOFT.
