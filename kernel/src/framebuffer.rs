use alloc::vec;
use alloc::vec::Vec;
use bootloader_api::info::{FrameBufferInfo, PixelFormat};
use core::fmt;
use noto_sans_mono_bitmap::{get_raster, FontWeight, RasterHeight};
use spin::Mutex;

use crate::cyrillic_font;

pub const FONT_WIDTH: usize = 9;
pub const FONT_HEIGHT: usize = 16;

pub const CURSOR_W: i32 = 12;
pub const CURSOR_H: i32 = 19;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const WHITE: Color = Color { r: 255, g: 255, b: 255 };
    pub const BLACK: Color = Color { r: 0, g: 0, b: 0 };
    pub const DESKTOP: Color = Color { r: 0, g: 128, b: 128 };
    pub const FACE: Color = Color { r: 192, g: 192, b: 192 };
    pub const HIGHLIGHT: Color = Color { r: 255, g: 255, b: 255 };
    pub const SHADOW: Color = Color { r: 128, g: 128, b: 128 };
    pub const DARK: Color = Color { r: 0, g: 0, b: 0 };
    pub const TITLE_ACTIVE: Color = Color { r: 0, g: 0, b: 128 };
    pub const TITLE_INACTIVE: Color = Color { r: 128, g: 128, b: 128 };
    pub const FIELD_BG: Color = Color { r: 255, g: 255, b: 255 };
    pub const TEXT: Color = Color { r: 0, g: 0, b: 0 };
    pub const TEXT_LIGHT: Color = Color { r: 255, g: 255, b: 255 };
}

fn blend(fg: Color, bg: Color, alpha: u8) -> Color {
    let a = alpha as u32;
    let inv = 255 - a;
    Color {
        r: ((fg.r as u32 * a + bg.r as u32 * inv) / 255) as u8,
        g: ((fg.g as u32 * a + bg.g as u32 * inv) / 255) as u8,
        b: ((fg.b as u32 * a + bg.b as u32 * inv) / 255) as u8,
    }
}

pub struct Writer {
    back: Vec<u8>,
    scene: Vec<u8>,
    screen: &'static mut [u8],
    pub width: usize,
    pub height: usize,
    pub stride: usize,
    pub bpp: usize,
    pub format: PixelFormat,
    pub cursor_x: usize,
    pub cursor_y: usize,
    pub fg: Color,
    pub bg: Color,
    last_cursor: Option<(i32, i32)>,
}

impl Writer {
    fn new(
        screen: &'static mut [u8],
        width: usize,
        height: usize,
        stride: usize,
        bpp: usize,
        format: PixelFormat,
    ) -> Self {
        let buf_len = stride * height * bpp;
        Self {
            back: vec![0u8; buf_len],
            scene: vec![0u8; buf_len],
            screen,
            width,
            height,
            stride,
            bpp,
            format,
            cursor_x: 0,
            cursor_y: 0,
            fg: Color::TEXT,
            bg: Color::FACE,
            last_cursor: None,
        }
    }

    pub fn flush_all(&mut self) {
        let len = self.screen.len().min(self.back.len());
        self.screen[..len].copy_from_slice(&self.back[..len]);
    }

    pub fn flush_rect(&mut self, x: i32, y: i32, w: i32, h: i32) {
        if w <= 0 || h <= 0 { return; }
        let x0 = x.max(0) as usize;
        let y0 = y.max(0) as usize;
        let x1 = ((x + w).min(self.width as i32)).max(0) as usize;
        let y1 = ((y + h).min(self.height as i32)).max(0) as usize;
        if x0 >= x1 || y0 >= y1 { return; }
        for yy in y0..y1 {
            let off = yy * self.stride * self.bpp + x0 * self.bpp;
            let len = (x1 - x0) * self.bpp;
            self.screen[off..off + len].copy_from_slice(&self.back[off..off + len]);
        }
    }

    fn copy_rect_from_scene(&mut self, x: i32, y: i32, w: i32, h: i32) {
        let x0 = x.max(0) as usize;
        let y0 = y.max(0) as usize;
        let x1 = ((x + w).min(self.width as i32)).max(0) as usize;
        let y1 = ((y + h).min(self.height as i32)).max(0) as usize;
        if x0 >= x1 || y0 >= y1 { return; }
        for yy in y0..y1 {
            let off = yy * self.stride * self.bpp + x0 * self.bpp;
            let len = (x1 - x0) * self.bpp;
            self.back[off..off + len].copy_from_slice(&self.scene[off..off + len]);
        }
    }

    pub fn end_scene(&mut self) {
        self.scene.copy_from_slice(&self.back);
        self.last_cursor = None;
    }

    pub fn move_cursor(
        &mut self,
        mx: i32,
        my: i32,
        pixels: &[(i32, i32, u8)],
        cw: i32,
        ch: i32,
    ) -> ((i32, i32, i32, i32), (i32, i32, i32, i32)) {
        let old_rect = if let Some((ox, oy)) = self.last_cursor {
            self.copy_rect_from_scene(ox - 1, oy - 1, cw + 2, ch + 2);
            (ox - 1, oy - 1, cw + 2, ch + 2)
        } else {
            (0, 0, 0, 0)
        };

        for &(dx, dy, c) in pixels {
            let x = mx + dx;
            let y = my + dy;
            if x < 0 || y < 0 { continue; }
            let color = if c == 0 { Color::DARK } else { Color::WHITE };
            self.set_pixel(x as usize, y as usize, color);
        }
        self.last_cursor = Some((mx, my));

        let new_rect = (mx - 1, my - 1, cw + 2, ch + 2);
        (old_rect, new_rect)
    }

    pub fn clear(&mut self) {
        self.fill_rect(0, 0, self.width, self.height, self.bg);
        self.cursor_x = 0;
        self.cursor_y = 0;
    }

    pub fn backspace(&mut self) {
        if self.cursor_x >= FONT_WIDTH {
            self.cursor_x -= FONT_WIDTH;
            for row in 0..FONT_HEIGHT {
                for col in 0..FONT_WIDTH {
                    self.set_pixel(self.cursor_x + col, self.cursor_y + row, self.bg);
                }
            }
        }
    }

    /// Возвращает цвет пикселя по координатам, или None если вне экрана.
    #[inline]
    pub fn get_pixel(&self, x: usize, y: usize) -> Option<Color> {
        if x >= self.width || y >= self.height { return None; }
        let off = y * self.stride * self.bpp + x * self.bpp;
        match self.format {
            PixelFormat::Rgb => Some(Color {
                r: self.back[off],
                g: self.back[off + 1],
                b: self.back[off + 2],
            }),
            PixelFormat::Bgr => Some(Color {
                r: self.back[off + 2],
                g: self.back[off + 1],
                b: self.back[off],
            }),
            PixelFormat::U8 => {
                let v = self.back[off];
                Some(Color { r: v, g: v, b: v })
            }
            _ => None,
        }
    }

    #[inline]
    pub fn set_pixel(&mut self, x: usize, y: usize, color: Color) {
        if x >= self.width || y >= self.height { return; }
        let offset = y * self.stride * self.bpp + x * self.bpp;
        match self.format {
            PixelFormat::Rgb => {
                self.back[offset] = color.r;
                self.back[offset + 1] = color.g;
                self.back[offset + 2] = color.b;
            }
            PixelFormat::Bgr => {
                self.back[offset] = color.b;
                self.back[offset + 1] = color.g;
                self.back[offset + 2] = color.r;
            }
            PixelFormat::U8 => {
                let gray = ((color.r as u32 + color.g as u32 + color.b as u32) / 3) as u8;
                self.back[offset] = gray;
            }
            _ => {}
        }
    }

    /// Alpha-наложение: blend существующего пикселя с `color` через `alpha` (0..255).
    pub fn blend_pixel(&mut self, x: usize, y: usize, color: Color, alpha: u8) {
        if x >= self.width || y >= self.height || alpha == 0 { return; }
        if alpha == 255 { self.set_pixel(x, y, color); return; }
        let offset = y * self.stride * self.bpp + x * self.bpp;
        let inv = 255 - alpha as u32;
        let a = alpha as u32;
        let (r, g, b) = match self.format {
            PixelFormat::Rgb => (self.back[offset], self.back[offset + 1], self.back[offset + 2]),
            PixelFormat::Bgr => (self.back[offset + 2], self.back[offset + 1], self.back[offset]),
            _ => return,
        };
        let nr = ((color.r as u32 * a + r as u32 * inv) / 255) as u8;
        let ng = ((color.g as u32 * a + g as u32 * inv) / 255) as u8;
        let nb = ((color.b as u32 * a + b as u32 * inv) / 255) as u8;
        match self.format {
            PixelFormat::Rgb => {
                self.back[offset] = nr;
                self.back[offset + 1] = ng;
                self.back[offset + 2] = nb;
            }
            PixelFormat::Bgr => {
                self.back[offset] = nb;
                self.back[offset + 1] = ng;
                self.back[offset + 2] = nr;
            }
            _ => {}
        }
    }

    pub fn fill_rect(&mut self, x: usize, y: usize, w: usize, h: usize, color: Color) {
        let x1 = (x + w).min(self.width);
        let y1 = (y + h).min(self.height);
        for yy in y..y1 {
            for xx in x..x1 {
                self.set_pixel(xx, yy, color);
            }
        }
    }

    /// Заливает прямоугольник цветом с alpha-смешением.
    pub fn fill_rect_blend(
        &mut self,
        x: usize,
        y: usize,
        w: usize,
        h: usize,
        color: Color,
        alpha: u8,
    ) {
        if alpha == 0 { return; }
        if alpha == 255 {
            self.fill_rect(x, y, w, h, color);
            return;
        }
        let x1 = (x + w).min(self.width);
        let y1 = (y + h).min(self.height);
        let a = alpha as u32;
        let inv = 255 - a;
        for yy in y..y1 {
            for xx in x..x1 {
                let off = yy * self.stride * self.bpp + xx * self.bpp;
                let (r, g, b) = match self.format {
                    PixelFormat::Rgb => (self.back[off], self.back[off + 1], self.back[off + 2]),
                    PixelFormat::Bgr => (self.back[off + 2], self.back[off + 1], self.back[off]),
                    _ => return,
                };
                let nr = ((color.r as u32 * a + r as u32 * inv) / 255) as u8;
                let ng = ((color.g as u32 * a + g as u32 * inv) / 255) as u8;
                let nb = ((color.b as u32 * a + b as u32 * inv) / 255) as u8;
                match self.format {
                    PixelFormat::Rgb => {
                        self.back[off] = nr;
                        self.back[off + 1] = ng;
                        self.back[off + 2] = nb;
                    }
                    PixelFormat::Bgr => {
                        self.back[off] = nb;
                        self.back[off + 1] = ng;
                        self.back[off + 2] = nr;
                    }
                    _ => {}
                }
            }
        }
    }

    /// Box blur по региону `(x, y, w, h)` радиусом `radius`.
    ///
    /// Реализация: два прохода (горизонтальный и вертикальный) — O(w·h·r),
    /// но с малым `r` (2–8) это быстрее прямого O(w·h·r²).
    pub fn blur_rect(&mut self, x: usize, y: usize, w: usize, h: usize, radius: usize) {
        if radius == 0 || w == 0 || h == 0 { return; }
        if self.bpp < 3 { return; }
        let x1 = (x + w).min(self.width);
        let y1 = (y + h).min(self.height);
        if x1 <= x || y1 <= y { return; }
        let rw = x1 - x;
        let rh = y1 - y;
        let r = radius;

        // Временные буферы: RGB 3 байта на пиксель.
        let mut tmp = vec![0u8; rw * rh * 3];
        let mut out = vec![0u8; rw * rh * 3];

        // Забираем исходные пиксели.
        for yy in 0..rh {
            for xx in 0..rw {
                let src_off = (y + yy) * self.stride * self.bpp + (x + xx) * self.bpp;
                let (r_, g_, b_) = match self.format {
                    PixelFormat::Rgb => (
                        self.back[src_off],
                        self.back[src_off + 1],
                        self.back[src_off + 2],
                    ),
                    PixelFormat::Bgr => (
                        self.back[src_off + 2],
                        self.back[src_off + 1],
                        self.back[src_off],
                    ),
                    _ => return,
                };
                let dst = (yy * rw + xx) * 3;
                tmp[dst] = r_;
                tmp[dst + 1] = g_;
                tmp[dst + 2] = b_;
            }
        }

        // Горизонтальный проход: tmp → out.
        for yy in 0..rh {
            for xx in 0..rw {
                let mut sr = 0u32;
                let mut sg = 0u32;
                let mut sb = 0u32;
                let mut n = 0u32;
                let lo = xx.saturating_sub(r);
                let hi = (xx + r + 1).min(rw);
                for k in lo..hi {
                    let s = (yy * rw + k) * 3;
                    sr += tmp[s] as u32;
                    sg += tmp[s + 1] as u32;
                    sb += tmp[s + 2] as u32;
                    n += 1;
                }
                let d = (yy * rw + xx) * 3;
                out[d] = (sr / n) as u8;
                out[d + 1] = (sg / n) as u8;
                out[d + 2] = (sb / n) as u8;
            }
        }

        // Вертикальный проход: out → tmp.
        for yy in 0..rh {
            for xx in 0..rw {
                let mut sr = 0u32;
                let mut sg = 0u32;
                let mut sb = 0u32;
                let mut n = 0u32;
                let lo = yy.saturating_sub(r);
                let hi = (yy + r + 1).min(rh);
                for k in lo..hi {
                    let s = (k * rw + xx) * 3;
                    sr += out[s] as u32;
                    sg += out[s + 1] as u32;
                    sb += out[s + 2] as u32;
                    n += 1;
                }
                let d = (yy * rw + xx) * 3;
                tmp[d] = (sr / n) as u8;
                tmp[d + 1] = (sg / n) as u8;
                tmp[d + 2] = (sb / n) as u8;
            }
        }

        // Возвращаем в back.
        for yy in 0..rh {
            for xx in 0..rw {
                let src = (yy * rw + xx) * 3;
                let dst_off = (y + yy) * self.stride * self.bpp + (x + xx) * self.bpp;
                match self.format {
                    PixelFormat::Rgb => {
                        self.back[dst_off] = tmp[src];
                        self.back[dst_off + 1] = tmp[src + 1];
                        self.back[dst_off + 2] = tmp[src + 2];
                    }
                    PixelFormat::Bgr => {
                        self.back[dst_off] = tmp[src + 2];
                        self.back[dst_off + 1] = tmp[src + 1];
                        self.back[dst_off + 2] = tmp[src];
                    }
                    _ => {}
                }
            }
        }
    }

    pub fn draw_border(
        &mut self,
        x: usize, y: usize, w: usize, h: usize,
        thickness: usize, color: Color,
    ) {
        if w == 0 || h == 0 || thickness == 0 { return; }
        let t = thickness.min(w).min(h);
        self.fill_rect(x, y, w, t, color);
        self.fill_rect(x, y + h - t, w, t, color);
        self.fill_rect(x, y, t, h, color);
        self.fill_rect(x + w - t, y, t, h, color);
    }

    pub fn bevel(&mut self, x: usize, y: usize, w: usize, h: usize, raised: bool) {
        if w < 2 || h < 2 { return; }
        let (tl, br) = if raised {
            (Color::HIGHLIGHT, Color::DARK)
        } else {
            (Color::SHADOW, Color::HIGHLIGHT)
        };
        self.fill_rect(x, y, w, 1, tl);
        self.fill_rect(x, y, 1, h, tl);
        self.fill_rect(x, y + h - 1, w, 1, br);
        self.fill_rect(x + w - 1, y, 1, h, br);

        if w > 2 && h > 2 {
            let tl2 = if raised { Color::FACE } else { Color::SHADOW };
            let br2 = if raised { Color::SHADOW } else { Color::FACE };
            self.fill_rect(x + 1, y + 1, w - 2, 1, tl2);
            self.fill_rect(x + 1, y + 1, 1, h - 2, tl2);
            self.fill_rect(x + 1, y + h - 2, w - 2, 1, br2);
            self.fill_rect(x + w - 2, y + 1, 1, h - 2, br2);
        }
    }

    pub fn gradient_v(
        &mut self,
        x: usize, y: usize, w: usize, h: usize,
        top: Color, bottom: Color,
    ) {
        if h == 0 { return; }
        let n = (h - 1) as u32;
        for i in 0..h {
            let t = i as u32;
            let inv = n - t;
            let c = if n == 0 {
                top
            } else {
                Color {
                    r: ((top.r as u32 * inv + bottom.r as u32 * t) / n) as u8,
                    g: ((top.g as u32 * inv + bottom.g as u32 * t) / n) as u8,
                    b: ((top.b as u32 * inv + bottom.b as u32 * t) / n) as u8,
                }
            };
            self.fill_rect(x, y + i, w, 1, c);
        }
    }

    pub fn fill_round_rect(
        &mut self, x: usize, y: usize, w: usize, h: usize,
        radius: usize, color: Color,
    ) {
        if w == 0 || h == 0 { return; }
        if w < 2 * radius || h < 2 * radius {
            self.fill_rect(x, y, w, h, color);
            return;
        }
        self.fill_rect(x + radius, y, w - 2 * radius, h, color);
        self.fill_rect(x, y + radius, radius, h - 2 * radius, color);
        self.fill_rect(x + w - radius, y + radius, radius, h - 2 * radius, color);

        let r = radius as i32;
        let r2 = r * r;
        for dy in 0..radius {
            for dx in 0..radius {
                let d2 = (dx as i32 - r).pow(2) + (dy as i32 - r).pow(2);
                if d2 <= r2 {
                    self.set_pixel(x + dx, y + dy, color);
                    self.set_pixel(x + w - 1 - dx, y + dy, color);
                    self.set_pixel(x + dx, y + h - 1 - dy, color);
                    self.set_pixel(x + w - 1 - dx, y + h - 1 - dy, color);
                }
            }
        }
    }

    pub fn fill_round_rect_aa(
        &mut self, x: usize, y: usize, w: usize, h: usize,
        radius: usize, color: Color,
    ) {
        if w == 0 || h == 0 || radius == 0 {
            self.fill_rect(x, y, w, h, color);
            return;
        }
        if w < 2 * radius || h < 2 * radius {
            self.fill_rect(x, y, w, h, color);
            return;
        }
        self.fill_rect(x + radius, y, w - 2 * radius, h, color);
        self.fill_rect(x, y + radius, radius, h - 2 * radius, color);
        self.fill_rect(x + w - radius, y + radius, radius, h - 2 * radius, color);

        let r = radius as f32;
        let cx = r - 0.5;
        let cy = r - 0.5;
        let dr = r + 0.5;
        let dr2 = dr * dr;
        let r05 = r - 0.5;
        let r05_2 = r05 * r05;
        let two_dr = 2.0 * dr;

        for dy in 0..radius {
            for dx in 0..radius {
                let ax = dx as f32 - cx;
                let ay = dy as f32 - cy;
                let d2 = ax * ax + ay * ay;
                let alpha: u8 = if d2 <= r05_2 {
                    255
                } else if d2 >= dr2 {
                    continue;
                } else {
                    let t = ((dr2 - d2) / two_dr) * 255.0;
                    if t <= 0.0 { continue; }
                    if t >= 255.0 { 255 } else { t as u8 }
                };
                self.blend_pixel(x + dx, y + dy, color, alpha);
                self.blend_pixel(x + w - 1 - dx, y + dy, color, alpha);
                self.blend_pixel(x + dx, y + h - 1 - dy, color, alpha);
                self.blend_pixel(x + w - 1 - dx, y + h - 1 - dy, color, alpha);
            }
        }
    }

    /// Закруглённый прямоугольник с AA **и** alpha-прозрачностью.
    pub fn fill_round_rect_aa_blend(
        &mut self, x: usize, y: usize, w: usize, h: usize,
        radius: usize, color: Color, base_alpha: u8,
    ) {
        if base_alpha == 0 { return; }
        if w == 0 || h == 0 || radius == 0 {
            self.fill_rect_blend(x, y, w, h, color, base_alpha);
            return;
        }
        if w < 2 * radius || h < 2 * radius {
            self.fill_rect_blend(x, y, w, h, color, base_alpha);
            return;
        }
        let mul = |a: u8| -> u8 {
            ((a as u32 * base_alpha as u32) / 255) as u8
        };

        self.fill_rect_blend(x + radius, y, w - 2 * radius, h, color, base_alpha);
        self.fill_rect_blend(x, y + radius, radius, h - 2 * radius, color, base_alpha);
        self.fill_rect_blend(x + w - radius, y + radius, radius, h - 2 * radius, color, base_alpha);

        let r = radius as f32;
        let cx = r - 0.5;
        let cy = r - 0.5;
        let dr = r + 0.5;
        let dr2 = dr * dr;
        let r05 = r - 0.5;
        let r05_2 = r05 * r05;
        let two_dr = 2.0 * dr;

        for dy in 0..radius {
            for dx in 0..radius {
                let ax = dx as f32 - cx;
                let ay = dy as f32 - cy;
                let d2 = ax * ax + ay * ay;
                let aa: u8 = if d2 <= r05_2 {
                    255
                } else if d2 >= dr2 {
                    continue;
                } else {
                    let t = ((dr2 - d2) / two_dr) * 255.0;
                    if t <= 0.0 { continue; }
                    if t >= 255.0 { 255 } else { t as u8 }
                };
                let a = mul(aa);
                self.blend_pixel(x + dx, y + dy, color, a);
                self.blend_pixel(x + w - 1 - dx, y + dy, color, a);
                self.blend_pixel(x + dx, y + h - 1 - dy, color, a);
                self.blend_pixel(x + w - 1 - dx, y + h - 1 - dy, color, a);
            }
        }
    }

    pub fn draw_text_at(&mut self, x: usize, y: usize, s: &str, fg: Color, bg: Color) {
        let sx = self.cursor_x;
        let sy = self.cursor_y;
        self.cursor_x = x;
        self.cursor_y = y;
        for c in s.chars() {
            self.draw_char_colored(c, fg, bg);
            self.cursor_x += FONT_WIDTH;
        }
        self.cursor_x = sx;
        self.cursor_y = sy;
    }

    pub fn text_width(s: &str) -> usize {
        s.chars().count() * FONT_WIDTH
    }

    fn newline(&mut self) {
        self.cursor_x = 0;
        self.cursor_y += FONT_HEIGHT;
    }

    fn write_char(&mut self, c: char) {
        match c {
            '\n' => self.newline(),
            _ => {
                if self.cursor_x + FONT_WIDTH > self.width {
                    self.newline();
                }
                self.draw_char_colored(c, self.fg, self.bg);
                self.cursor_x += FONT_WIDTH;
            }
        }
    }

    fn draw_char_colored(&mut self, c: char, fg: Color, bg: Color) {
        if let Some(bitmap) = get_raster(c, FontWeight::Regular, RasterHeight::Size16) {
            let w = bitmap.width();
            let h = bitmap.height();
            let raster = bitmap.raster();
            for y in 0..h {
                let row = raster[y];
                for x in 0..w {
                    let alpha = row[x];
                    if alpha == 0 { continue; }
                    let color = blend(fg, bg, alpha);
                    self.set_pixel(self.cursor_x + x, self.cursor_y + y, color);
                }
            }
            return;
        }

        if let Some(bitmap) = cyrillic_font::get_cyrillic(c) {
            self.draw_8x8(bitmap, fg, bg);
            return;
        }

        if let Some(bitmap) = get_raster('?', FontWeight::Regular, RasterHeight::Size16) {
            let w = bitmap.width();
            let h = bitmap.height();
            let raster = bitmap.raster();
            for y in 0..h {
                let row = raster[y];
                for x in 0..w {
                    let alpha = row[x];
                    if alpha == 0 { continue; }
                    let color = blend(fg, bg, alpha);
                    self.set_pixel(self.cursor_x + x, self.cursor_y + y, color);
                }
            }
        }
    }

    fn draw_8x8(&mut self, bitmap: [u8; 8], fg: Color, bg: Color) {
        for (row, byte) in bitmap.iter().enumerate() {
            for col in 0..8usize {
                let on = (byte >> (7 - col)) & 1 == 1;
                let color = if on { fg } else { bg };
                let x0 = self.cursor_x + col;
                let y0 = self.cursor_y + row * 2;
                let y1 = y0 + 1;
                self.set_pixel(x0, y0, color);
                self.set_pixel(x0, y1, color);
                if col == 7 {
                    self.set_pixel(self.cursor_x + 8, y0, color);
                    self.set_pixel(self.cursor_x + 8, y1, color);
                }
            }
        }
    }
}

impl fmt::Write for Writer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.chars() {
            self.write_char(c);
        }
        Ok(())
    }
}

static WRITER: Mutex<Option<Writer>> = Mutex::new(None);

pub fn init(info: FrameBufferInfo, buffer: &'static mut [u8]) {
    let w = Writer::new(
        buffer,
        info.width,
        info.height,
        info.stride,
        info.bytes_per_pixel,
        info.pixel_format,
    );
    *WRITER.lock() = Some(w);
}

pub fn with_writer<R, F: FnOnce(&mut Writer) -> R>(f: F) -> R {
    let mut guard = WRITER.lock();
    let w = guard.as_mut().expect("framebuffer не инициализирован");
    f(w)
}

pub fn clear() { with_writer(|w| w.clear()); }
pub fn backspace() { with_writer(|w| w.backspace()); }
pub fn flush() { with_writer(|w| w.flush_all()); }

pub fn _print(args: fmt::Arguments) {
    use core::fmt::Write;
    with_writer(|w| {
        let _ = w.write_fmt(args);
    });
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ($crate::framebuffer::_print(format_args!($($arg)*)));
}

#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}