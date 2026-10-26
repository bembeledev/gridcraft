//! A minimal PNG writer for the pictures an `.xls` import converts (RGBA, 8 bits).

use miniz_oxide::deflate::compress_to_vec_zlib;

fn crc32(chunk: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in chunk {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn push_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let from = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[from..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// Encodes `rgba` (`width * height * 4` bytes, not premultiplied) as a PNG, or `None` when the
/// size doesn't match.
pub(super) fn encode(width: u32, height: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    let row = (width as usize).checked_mul(4)?;
    if width == 0 || height == 0 || rgba.len() != row.checked_mul(height as usize)? {
        return None;
    }
    let mut raw = Vec::with_capacity((row + 1) * height as usize);
    for line in rgba.chunks_exact(row) {
        raw.push(0); // filter: none
        raw.extend_from_slice(line);
    }
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8 bits, RGBA, deflate, no filter method, no interlace
    push_chunk(&mut out, b"IHDR", &ihdr);
    push_chunk(&mut out, b"IDAT", &compress_to_vec_zlib(&raw, 6));
    push_chunk(&mut out, b"IEND", &[]);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_a_png_the_image_crate_reads_back() {
        let rgba = [255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 255, 0, 0, 0, 0];
        let png = encode(2, 2, &rgba).unwrap();
        let img = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!(img.as_raw().as_slice(), &rgba);
        assert!(encode(2, 2, &rgba[..8]).is_none());
        assert!(encode(0, 2, &[]).is_none());
    }
}
