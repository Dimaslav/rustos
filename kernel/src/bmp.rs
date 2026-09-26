//! Минимальный декодер BMP.
//!
//! Поддерживает:
//! - 24-bit uncompressed (BI_RGB, biBitCount=24)
//! - 32-bit uncompressed (BI_RGB, biBitCount=32) — alpha игнорируется
//!
//! Результат: `pixels` — top-down RGB (3 байта на пиксель),
//! порядок как в памяти: row0[0..w*3], row1[0..w*3], ...

use alloc::vec;
use alloc::vec::Vec;

pub struct Bmp {
    pub width: usize,
    pub height: usize,
    /// RGB, 3 байта на пиксель, top-down.
    pub pixels: Vec<u8>,
}

pub fn decode(data: &[u8]) -> Option<Bmp> {
    if data.len() < 54 || &data[0..2] != b"BM" {
        return None;
    }

    let data_offset =
        u32::from_le_bytes([data[10], data[11], data[12], data[13]]) as usize;

    // DIB header size (must be >= 40 for BITMAPINFOHEADER).
    let dib_size = u32::from_le_bytes([data[14], data[15], data[16], data[17]]);
    if dib_size < 40 {
        return None;
    }

    let width = i32::from_le_bytes([data[18], data[19], data[20], data[21]]);
    let height_signed = i32::from_le_bytes([data[22], data[23], data[24], data[25]]);
    let planes = u16::from_le_bytes([data[26], data[27]]);
    let bpp = u16::from_le_bytes([data[28], data[29]]);
    let compression =
        u32::from_le_bytes([data[30], data[31], data[32], data[33]]);

    if planes != 1 || compression != 0 {
        return None;
    }
    if bpp != 24 && bpp != 32 {
        return None;
    }
    if width <= 0 {
        return None;
    }

    let w = width as usize;
    let h = height_signed.unsigned_abs() as usize;
    let bottom_up = height_signed > 0;

    let bpp_bytes = (bpp / 8) as usize;
    // Строка выровнена на 4 байта.
    let row_raw = w * bpp_bytes;
    let row_padded = (row_raw + 3) & !3;
    let total_needed = data_offset + row_padded * h;
    if data.len() < total_needed {
        return None;
    }

    let mut pixels = vec![0u8; w * h * 3];

    for row in 0..h {
        let src_row = if bottom_up { h - 1 - row } else { row };
        let src_start = data_offset + src_row * row_padded;
        let dst_start = row * w * 3;
        for x in 0..w {
            let s = src_start + x * bpp_bytes;
            let d = dst_start + x * 3;
            // BMP хранит BGR(A).
            let b = data[s];
            let g = data[s + 1];
            let r = data[s + 2];
            pixels[d] = r;
            pixels[d + 1] = g;
            pixels[d + 2] = b;
        }
    }

    Some(Bmp { width: w, height: h, pixels })
}