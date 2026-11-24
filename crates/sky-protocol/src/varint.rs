use crate::error::{ProtocolError, ProtocolResult};

/// Returns the encoded byte length (1..=5) of a 32-bit Minecraft `VarInt`.
pub const fn varint_len(value: i32) -> usize {
    let mut v = value as u32;
    let mut len = 1;
    while (v & !0x7F) != 0 {
        v >>= 7;
        len += 1;
    }
    len
}

/// Encodes `value` as a Minecraft `VarInt` into `out` (at most 5 bytes).
pub fn write_varint(value: i32, out: &mut Vec<u8>) {
    let mut v = value as u32;
    while (v & !0x7F) != 0 {
        out.push(((v & 0x7F) as u8) | 0x80);
        v >>= 7;
    }
    out.push((v & 0x7F) as u8);
}

/// Decodes a Minecraft `VarInt` from `slice`, returning `(value, bytes_consumed)`.
pub fn read_varint(slice: &[u8]) -> ProtocolResult<(i32, usize)> {
    let mut num_read = 0usize;
    let mut result = 0u32;

    loop {
        let Some(&byte) = slice.get(num_read) else {
            return Err(ProtocolError::UnexpectedEof {
                needed: num_read + 1,
                available: slice.len(),
            });
        };
        if num_read >= 5 {
            return Err(ProtocolError::VarIntTooLarge);
        }
        let value = u32::from(byte & 0x7F);
        result |= value << (7 * num_read);
        num_read += 1;

        if (byte & 0x80) == 0 {
            return Ok((result as i32, num_read));
        }
    }
}

/// Encodes `value` as a Minecraft `VarLong` into `out` (at most 10 bytes).
pub fn write_varlong(value: i64, out: &mut Vec<u8>) {
    let mut v = value as u64;
    while (v & !0x7F) != 0 {
        out.push(((v & 0x7F) as u8) | 0x80);
        v >>= 7;
    }
    out.push((v & 0x7F) as u8);
}

/// Decodes a Minecraft `VarLong` from `slice`, returning `(value, bytes_consumed)`.
pub fn read_varlong(slice: &[u8]) -> ProtocolResult<(i64, usize)> {
    let mut num_read = 0usize;
    let mut result = 0u64;

    loop {
        let Some(&byte) = slice.get(num_read) else {
            return Err(ProtocolError::UnexpectedEof {
                needed: num_read + 1,
                available: slice.len(),
            });
        };
        if num_read >= 10 {
            return Err(ProtocolError::VarLongTooLarge);
        }
        let value = u64::from(byte & 0x7F);
        result |= value << (7 * num_read);
        num_read += 1;

        if (byte & 0x80) == 0 {
            return Ok((result as i64, num_read));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_roundtrip_known_vectors() {
        let cases: &[(i32, &[u8])] = &[
            (0, &[0x00]),
            (1, &[0x01]),
            (2, &[0x02]),
            (127, &[0x7f]),
            (128, &[0x80, 0x01]),
            (255, &[0xff, 0x01]),
            (25565, &[0xdd, 0xc7, 0x01]),
            (777, &[0x89, 0x06]),
            (2097151, &[0xff, 0xff, 0x7f]),
            (2147483647, &[0xff, 0xff, 0xff, 0xff, 0x07]),
            (-1, &[0xff, 0xff, 0xff, 0xff, 0x0f]),
            (-2147483648, &[0x80, 0x80, 0x80, 0x80, 0x08]),
        ];

        for &(val, expected) in cases {
            let mut buf = Vec::new();
            write_varint(val, &mut buf);
            assert_eq!(buf.as_slice(), expected, "encode {val}");
            assert_eq!(varint_len(val), expected.len());
            let (decoded, used) = read_varint(&buf).expect("decode varint");
            assert_eq!(decoded, val);
            assert_eq!(used, expected.len());
        }
    }

    #[test]
    fn rejects_six_byte_varint() {
        let bad = [0x80, 0x80, 0x80, 0x80, 0x80, 0x01];
        assert!(matches!(
            read_varint(&bad),
            Err(ProtocolError::VarIntTooLarge)
        ));
    }

    #[test]
    fn varlong_roundtrip() {
        for val in [0i64, 1, -1, i64::MAX, i64::MIN, 987_654_321_123] {
            let mut buf = Vec::new();
            write_varlong(val, &mut buf);
            let (decoded, used) = read_varlong(&buf).expect("decode varlong");
            assert_eq!(decoded, val);
            assert_eq!(used, buf.len());
        }
    }
}
