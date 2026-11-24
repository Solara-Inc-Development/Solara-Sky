use crate::error::{ProtocolError, ProtocolResult};
use crate::varint::{read_varint, read_varlong, write_varint, write_varlong};
use uuid::Uuid;

/// Zero-copy bounded cursor over an incoming packet payload.
#[derive(Debug, Clone)]
pub struct PacketReader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> PacketReader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }

    pub fn ensure_empty(&self) -> ProtocolResult<()> {
        let rem = self.remaining();
        if rem == 0 {
            Ok(())
        } else {
            Err(ProtocolError::TrailingBytes(rem))
        }
    }

    pub fn read_exact(&mut self, len: usize) -> ProtocolResult<&'a [u8]> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or(ProtocolError::UnexpectedEof {
                needed: len,
                available: self.remaining(),
            })?;
        if end > self.buf.len() {
            return Err(ProtocolError::UnexpectedEof {
                needed: len,
                available: self.remaining(),
            });
        }
        let slice = &self.buf[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    pub fn read_remaining(&mut self) -> &'a [u8] {
        let slice = &self.buf[self.pos..];
        self.pos = self.buf.len();
        slice
    }

    pub fn read_u8(&mut self) -> ProtocolResult<u8> {
        Ok(self.read_exact(1)?[0])
    }

    pub fn read_i8(&mut self) -> ProtocolResult<i8> {
        Ok(self.read_u8()? as i8)
    }

    pub fn read_bool(&mut self) -> ProtocolResult<bool> {
        Ok(self.read_u8()? != 0)
    }

    pub fn read_u16(&mut self) -> ProtocolResult<u16> {
        let b = self.read_exact(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    pub fn read_i16(&mut self) -> ProtocolResult<i16> {
        let b = self.read_exact(2)?;
        Ok(i16::from_be_bytes([b[0], b[1]]))
    }

    pub fn read_i32(&mut self) -> ProtocolResult<i32> {
        let b = self.read_exact(4)?;
        Ok(i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn read_i64(&mut self) -> ProtocolResult<i64> {
        let b = self.read_exact(8)?;
        Ok(i64::from_be_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    pub fn read_f32(&mut self) -> ProtocolResult<f32> {
        let b = self.read_exact(4)?;
        Ok(f32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn read_f64(&mut self) -> ProtocolResult<f64> {
        let b = self.read_exact(8)?;
        Ok(f64::from_be_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    pub fn read_varint(&mut self) -> ProtocolResult<i32> {
        let (val, used) = read_varint(&self.buf[self.pos..])?;
        self.pos += used;
        Ok(val)
    }

    pub fn read_varlong(&mut self) -> ProtocolResult<i64> {
        let (val, used) = read_varlong(&self.buf[self.pos..])?;
        self.pos += used;
        Ok(val)
    }

    pub fn read_usize_bounded(&mut self, max: usize) -> ProtocolResult<usize> {
        let raw = self.read_varint()?;
        if raw < 0 {
            return Err(ProtocolError::NegativeLength(raw));
        }
        let len = raw as usize;
        if len > max {
            return Err(ProtocolError::LengthTooLarge { len, max });
        }
        Ok(len)
    }

    pub fn read_byte_array(&mut self, max_len: usize) -> ProtocolResult<Vec<u8>> {
        let len = self.read_usize_bounded(max_len)?;
        Ok(self.read_exact(len)?.to_vec())
    }

    /// Reads a UTF-8 string bounded by `max_utf16_units` (matching Minecraft's `Utf8String.read`).
    pub fn read_string(&mut self, max_utf16_units: usize) -> ProtocolResult<String> {
        let max_bytes = max_utf16_units.saturating_mul(3);
        let byte_len = self.read_usize_bounded(max_bytes)?;
        let bytes = self.read_exact(byte_len)?;
        let s = std::str::from_utf8(bytes).map_err(|_| ProtocolError::InvalidUtf8)?;
        let utf16_len = s.encode_utf16().count();
        if utf16_len > max_utf16_units {
            return Err(ProtocolError::StringTooLong {
                chars: utf16_len,
                max: max_utf16_units,
            });
        }
        Ok(s.to_owned())
    }

    pub fn read_uuid(&mut self) -> ProtocolResult<Uuid> {
        let b = self.read_exact(16)?;
        let mut arr = [0u8; 16];
        arr.copy_from_slice(b);
        Ok(Uuid::from_bytes(arr))
    }

    /// Reads a packed 64-bit Minecraft block position `(x, y, z)`.
    pub fn read_block_pos(&mut self) -> ProtocolResult<(i32, i32, i32)> {
        let val = self.read_i64()?;
        let x = (val >> 38) as i32;
        let y = ((val << 52) >> 52) as i32;
        let z = ((val << 26) >> 38) as i32;
        Ok((x, y, z))
    }
}

/// Reusable buffer writer for outgoing packet payloads.
#[derive(Debug, Default, Clone)]
pub struct PacketWriter {
    buf: Vec<u8>,
}

impl PacketWriter {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self {
            buf: Vec::with_capacity(cap),
        }
    }

    pub fn into_inner(self) -> Vec<u8> {
        self.buf
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.buf
    }

    pub fn write_slice(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    pub fn write_u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    pub fn write_i8(&mut self, v: i8) {
        self.buf.push(v as u8);
    }

    pub fn write_bool(&mut self, v: bool) {
        self.buf.push(u8::from(v));
    }

    pub fn write_u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    pub fn write_i16(&mut self, v: i16) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    pub fn write_i32(&mut self, v: i32) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    pub fn write_i64(&mut self, v: i64) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    pub fn write_f32(&mut self, v: f32) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    pub fn write_f64(&mut self, v: f64) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    pub fn write_varint(&mut self, v: i32) {
        write_varint(v, &mut self.buf);
    }

    pub fn write_varlong(&mut self, v: i64) {
        write_varlong(v, &mut self.buf);
    }

    pub fn write_byte_array(&mut self, bytes: &[u8]) {
        self.write_varint(bytes.len() as i32);
        self.write_slice(bytes);
    }

    pub fn write_string(&mut self, s: &str) {
        self.write_varint(s.len() as i32);
        self.write_slice(s.as_bytes());
    }

    pub fn write_uuid(&mut self, uuid: Uuid) {
        self.write_slice(uuid.as_bytes());
    }

    pub fn write_block_pos(&mut self, x: i32, y: i32, z: i32) {
        let packed = (((x as i64) & 0x3FF_FFFF) << 38)
            | (((z as i64) & 0x3FF_FFFF) << 12)
            | ((y as i64) & 0xFFF);
        self.write_i64(packed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitives_and_block_pos_roundtrip() {
        let mut w = PacketWriter::new();
        let id = Uuid::new_v4();
        w.write_bool(true);
        w.write_u16(25565);
        w.write_string("Solara-Sky 26.3");
        w.write_uuid(id);
        w.write_block_pos(-12345, -64, 98765);

        let mut r = PacketReader::new(w.as_slice());
        assert!(r.read_bool().expect("bool"));
        assert_eq!(r.read_u16().expect("u16"), 25565);
        assert_eq!(r.read_string(32).expect("str"), "Solara-Sky 26.3");
        assert_eq!(r.read_uuid().expect("uuid"), id);
        assert_eq!(r.read_block_pos().expect("pos"), (-12345, -64, 98765));
        r.ensure_empty().expect("empty");
    }

    #[test]
    fn rejects_overlong_string() {
        let mut w = PacketWriter::new();
        w.write_string("abcdef");
        let mut r = PacketReader::new(w.as_slice());
        assert!(r.read_string(3).is_err());
    }
}
