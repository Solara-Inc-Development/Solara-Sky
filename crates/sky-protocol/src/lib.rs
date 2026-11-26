//! Minecraft: Java Edition 26.3 (protocol 777) wire protocol primitives and packets for Solara-Sky.
//!
//! All decoders enforce strict length and depth bounds and never panic on untrusted input.

pub mod buf;
pub mod config_play;
pub mod crypto;
pub mod entities;
pub mod error;
pub mod frame;
pub mod generated_packets;
pub mod generated_registries;
pub mod nbt;
pub mod packets;
pub mod survival;
pub mod varint;
pub mod velocity;

pub use error::{ProtocolError, ProtocolResult};

/// Targeted Minecraft: Java Edition version name.
pub const MINECRAFT_VERSION: &str = "26.3";

/// Targeted Minecraft: Java Edition protocol number (26.3 "Wilderness Bound").
pub const PROTOCOL_VERSION: i32 = 777;
