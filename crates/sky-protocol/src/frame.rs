use crate::error::{ProtocolError, ProtocolResult};
use crate::varint::{read_varint, varint_len, write_varint};
use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use std::io::{Read, Write};

/// Hard wire limit on a single framed packet (2 MiB).
pub const MAX_WIRE_FRAME_BYTES: usize = 2 * 1024 * 1024;

/// Hard limit on an uncompressed packet payload (8 MiB, matching Minecraft's protocol bound).
pub const MAX_UNCOMPRESSED_BYTES: usize = 8 * 1024 * 1024;

/// Frames `(packet_id, body)` into a complete wire packet with optional zlib compression.
pub fn encode_frame(
    packet_id: i32,
    body: &[u8],
    compression_threshold: Option<usize>,
) -> ProtocolResult<Vec<u8>> {
    let mut uncompressed = Vec::with_capacity(varint_len(packet_id) + body.len());
    write_varint(packet_id, &mut uncompressed);
    uncompressed.extend_from_slice(body);

    match compression_threshold {
        None => {
            let mut out =
                Vec::with_capacity(varint_len(uncompressed.len() as i32) + uncompressed.len());
            write_varint(uncompressed.len() as i32, &mut out);
            out.extend_from_slice(&uncompressed);
            Ok(out)
        }
        Some(threshold) => {
            if uncompressed.len() < threshold {
                let data_len = 0i32;
                let total_len = varint_len(data_len) + uncompressed.len();
                let mut out = Vec::with_capacity(varint_len(total_len as i32) + total_len);
                write_varint(total_len as i32, &mut out);
                write_varint(data_len, &mut out);
                out.extend_from_slice(&uncompressed);
                Ok(out)
            } else {
                let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
                encoder
                    .write_all(&uncompressed)
                    .map_err(|e| ProtocolError::Compression(e.to_string()))?;
                let compressed = encoder
                    .finish()
                    .map_err(|e| ProtocolError::Compression(e.to_string()))?;
                let data_len = uncompressed.len() as i32;
                let total_len = varint_len(data_len) + compressed.len();
                let mut out = Vec::with_capacity(varint_len(total_len as i32) + total_len);
                write_varint(total_len as i32, &mut out);
                write_varint(data_len, &mut out);
                out.extend_from_slice(&compressed);
                Ok(out)
            }
        }
    }
}

/// Decodes an already length-delimited frame body into `(packet_id, payload_bytes)`.
pub fn decode_frame_body(
    frame_body: &[u8],
    compression_enabled: bool,
) -> ProtocolResult<(i32, Vec<u8>)> {
    if frame_body.len() > MAX_WIRE_FRAME_BYTES {
        return Err(ProtocolError::PacketTooLarge {
            len: frame_body.len(),
            max: MAX_WIRE_FRAME_BYTES,
        });
    }

    if !compression_enabled {
        let (packet_id, used) = read_varint(frame_body)?;
        return Ok((packet_id, frame_body[used..].to_vec()));
    }

    let (data_len_raw, header_used) = read_varint(frame_body)?;
    if data_len_raw < 0 {
        return Err(ProtocolError::NegativeLength(data_len_raw));
    }
    let data_len = data_len_raw as usize;
    let remaining = &frame_body[header_used..];

    if data_len == 0 {
        let (packet_id, used) = read_varint(remaining)?;
        return Ok((packet_id, remaining[used..].to_vec()));
    }

    if data_len > MAX_UNCOMPRESSED_BYTES {
        return Err(ProtocolError::PacketTooLarge {
            len: data_len,
            max: MAX_UNCOMPRESSED_BYTES,
        });
    }

    // Stream-bound decompression so a zip-bomb cannot allocate beyond MAX_UNCOMPRESSED_BYTES + 1.
    let decoder = ZlibDecoder::new(remaining);
    let mut limited = decoder.take((MAX_UNCOMPRESSED_BYTES + 1) as u64);
    let mut decompressed = Vec::with_capacity(data_len.min(64 * 1024));
    limited
        .read_to_end(&mut decompressed)
        .map_err(|e| ProtocolError::Compression(e.to_string()))?;

    if decompressed.len() != data_len {
        return Err(ProtocolError::Compression(format!(
            "decompressed size {} did not match declared {data_len}",
            decompressed.len()
        )));
    }

    let (packet_id, used) = read_varint(&decompressed)?;
    Ok((packet_id, decompressed[used..].to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uncompressed_and_compressed_frames_roundtrip() {
        let payload = vec![0xAB; 1024];

        // Uncompressed
        let framed = encode_frame(0x02, &payload, None).expect("encode uncompressed");
        let (len, l_used) = read_varint(&framed).expect("frame len");
        assert_eq!(len as usize, framed.len() - l_used);
        let (pid, body) = decode_frame_body(&framed[l_used..], false).expect("decode uncompressed");
        assert_eq!(pid, 0x02);
        assert_eq!(body, payload);

        // Below threshold (compression enabled, data_len = 0)
        let below = encode_frame(0x02, &[1, 2, 3], Some(256)).expect("encode below threshold");
        let (_, bl_used) = read_varint(&below).expect("len");
        let (pid2, body2) = decode_frame_body(&below[bl_used..], true).expect("decode below");
        assert_eq!(pid2, 0x02);
        assert_eq!(body2, vec![1, 2, 3]);

        // Above threshold (zlib compressed)
        let above = encode_frame(0x05, &payload, Some(64)).expect("encode compressed");
        assert!(above.len() < payload.len());
        let (_, al_used) = read_varint(&above).expect("len");
        let (pid3, body3) = decode_frame_body(&above[al_used..], true).expect("decode above");
        assert_eq!(pid3, 0x05);
        assert_eq!(body3, payload);
    }
}
