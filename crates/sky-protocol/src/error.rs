use thiserror::Error;

pub type ProtocolResult<T> = Result<T, ProtocolError>;

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("unexpected end of packet buffer (needed {needed} bytes, had {available})")]
    UnexpectedEof { needed: usize, available: usize },

    #[error("VarInt exceeds maximum length of 5 bytes")]
    VarIntTooLarge,

    #[error("VarLong exceeds maximum length of 10 bytes")]
    VarLongTooLarge,

    #[error("negative length field: {0}")]
    NegativeLength(i32),

    #[error("length {len} exceeds maximum allowed {max}")]
    LengthTooLarge { len: usize, max: usize },

    #[error("invalid UTF-8 string in packet")]
    InvalidUtf8,

    #[error("string length {chars} UTF-16 units exceeds maximum {max}")]
    StringTooLong { chars: usize, max: usize },

    #[error("packet frame length {len} exceeds limit {max}")]
    PacketTooLarge { len: usize, max: usize },

    #[error("zlib compression/decompression error: {0}")]
    Compression(String),

    #[error("invalid NBT tag id: {0}")]
    InvalidNbtTag(u8),

    #[error("NBT nesting depth exceeded maximum of {0}")]
    NbtDepthExceeded(usize),

    #[error("invalid connection intent id: {0}")]
    InvalidIntent(i32),

    #[error("unexpected packet id 0x{0:02X} for current state")]
    UnexpectedPacketId(i32),

    #[error("invalid Velocity forwarding HMAC signature")]
    InvalidVelocitySignature,

    #[error("unsupported Velocity forwarding version: {0}")]
    UnsupportedVelocityVersion(i32),

    #[error("malformed BungeeCord handshake address payload")]
    InvalidBungeePayload,

    #[error("trailing {0} unread bytes in packet")]
    TrailingBytes(usize),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),
}
