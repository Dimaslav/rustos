//! PNG декодер. Поддержка: color types 0 (gray), 2 (RGB), 3 (palette),
//! 6 (RGBA), bit depth 8, non-interlaced. Использует miniz_oxide для DEFLATE.

use alloc::vec;
use alloc::vec::Vec;

pub struct Png {
    pub width: usize,
    pub height: usize,
    /// RGB, top-down.
    pub pixels: Vec<u8>,
}

const SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

pub fn decode(data: &[u8]) -> Option<Png> {
    if data.len() < 8 || data[0..8] != SIGNATURE {
        return None;
    }
    let mut pos = 8usize;

    let mut width = 0usize;
    let mut height = 0usize;
    let mut bit_depth = 0u8;
    let mut color_type = 0u8;
    let mut idat: Vec<u8> = Vec::new();
    let mut palette: Vec<(u8, u8, u8)> = Vec::new();

    while pos + 8 <= data.len() {
        let len = u32::from_be_bytes([
            data[pos],
            data[pos + 1],
            data[pos + 2],
            data[pos + 3],
        ]) as usize;
        let ctype = &data[pos + 4..pos + 8];
        pos += 8;
        if pos + len + 4 > data.len() {
            return None;
        }
        let chunk = &data[pos..pos + len];
        pos += len + 4; // skip data + CRC

        match ctype {
            b"IHDR" => {
                if chunk.len() < 13 {
                    return None;
                }
                width = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) as usize;
                height = u32::from_be_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]) as usize;
                bit_depth = chunk[8];
                color_type = chunk[9];
                let interlace = chunk[12];
                if width == 0 || height == 0 || interlace != 0 {
                    return None;
                }
            }
            b"PLTE" => {
                if chunk.len() % 3 != 0 {
                    return None;
                }
                palette.clear();
                let mut i = 0;
                while i + 2 < chunk.len() {
                    palette.push((chunk[i], chunk[i + 1], chunk[i + 2]));
                    i += 3;
                }
            }
            b"IDAT" => idat.extend_from_slice(chunk),
            b"IEND" => break,
            _ => {}
        }
    }

    if idat.is_empty() || width == 0 || height == 0 {
        return None;
    }

    let raw = miniz_oxide::inflate::decompress_to_vec(&idat).ok()?;

    let channels: usize = match color_type {
        0 => 1,
        2 => 3,
        3 => 1,
        6 => 4,
        _ => return None,
    };
    if bit_depth != 8 {
        return None;
    }

    let bpp = channels;
    let stride = width * bpp;
    if raw.len() < (stride + 1) * height {
        return None;
    }

    let mut img = vec![0u8; stride * height];
    let mut prev = vec![0u8; stride];
    for y in 0..height {
        let off = y * (stride + 1);
        let filter = raw[off];
        let line = &raw[off + 1..off + 1 + stride];
        let dst_off = y * stride;
        unfilter(filter, line, &prev, &mut img[dst_off..dst_off + stride], bpp)?;
        prev.copy_from_slice(&img[dst_off..dst_off + stride]);
    }

    let mut out = vec![0u8; width * height * 3];
    match color_type {
        0 => {
            for i in 0..width * height {
                let v = img[i];
                out[i * 3] = v;
                out[i * 3 + 1] = v;
                out[i * 3 + 2] = v;
            }
        }
        2 => out.copy_from_slice(&img),
        3 => {
            for i in 0..width * height {
                let idx = img[i] as usize;
                if idx >= palette.len() {
                    return None;
                }
                let (r, g, b) = palette[idx];
                out[i * 3] = r;
                out[i * 3 + 1] = g;
                out[i * 3 + 2] = b;
            }
        }
        6 => {
            for i in 0..width * height {
                out[i * 3] = img[i * 4];
                out[i * 3 + 1] = img[i * 4 + 1];
                out[i * 3 + 2] = img[i * 4 + 2];
            }
        }
        _ => return None,
    }

    Some(Png { width, height, pixels: out })
}

fn unfilter(filter: u8, line: &[u8], prev: &[u8], dst: &mut [u8], bpp: usize) -> Option<()> {
    if line.len() != dst.len() || prev.len() != dst.len() {
        return None;
    }
    match filter {
        0 => dst.copy_from_slice(line),
        1 => {
            for i in 0..line.len() {
                let a = if i >= bpp { dst[i - bpp] } else { 0 };
                dst[i] = line[i].wrapping_add(a);
            }
        }
        2 => {
            for i in 0..line.len() {
                dst[i] = line[i].wrapping_add(prev[i]);
            }
        }
        3 => {
            for i in 0..line.len() {
                let a = if i >= bpp { dst[i - bpp] as u32 } else { 0 };
                let b = prev[i] as u32;
                dst[i] = line[i].wrapping_add(((a + b) / 2) as u8);
            }
        }
        4 => {
            for i in 0..line.len() {
                let a = if i >= bpp { dst[i - bpp] as i32 } else { 0 };
                let b = prev[i] as i32;
                let c = if i >= bpp { prev[i - bpp] as i32 } else { 0 };
                let p = a + b - c;
                let pa = (p - a).abs();
                let pb = (p - b).abs();
                let pc = (p - c).abs();
                let pred = if pa <= pb && pa <= pc {
                    a
                } else if pb <= pc {
                    b
                } else {
                    c
                };
                dst[i] = line[i].wrapping_add(pred as u8);
            }
        }
        _ => return None,
    }
    Some(())
}