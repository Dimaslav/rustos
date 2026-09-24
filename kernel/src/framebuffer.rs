use bootloader_api::info::{FrameBufferInfo, PixelFormat};
use core::fmt;
use noto_sans_mono_bitmap::{get_raster, FontWeight, RasterHeight};
use spin::Mutex;

/// Размер знакоместа. Noto Sans Mono Size16 — 9×16 пикселей.
pub const FONT_WIDTH: usize = 9;
pub const FONT_HEIGHT: usize = 16;

#[derive(Clone, Copy)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const WHITE: Color = Color { r: 255, g: 255, b: 255 };
    pub const BLACK: Color = Color { r: 0, g: 0, b: 0 };
}

/// Линейная интерполяция цвета по alpha (0..=255).
/// alpha=255 → чистый fg, alpha=0 → чистый bg.
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
    buffer: &'static mut [u8],
    width: usize,
    height: usize,
    stride: usize,
    bpp: usize,
    format: PixelFormat,
    cursor_x: usize,
    cursor_y: usize,
    fg: Color,
    bg: Color,
}

impl Writer {
    fn new(
        buffer: &'static mut [u8],
        width: usize,
        height: usize,
        stride: usize,
        bpp: usize,
        format: PixelFormat,
    ) -> Self {
        Self {
            buffer,
            width,
            height,
            stride,
            bpp,
            format,
            cursor_x: 0,
            cursor_y: 0,
            fg: Color::WHITE,
            bg: Color::BLACK,
        }
    }

    pub fn clear(&mut self) {
        for y in 0..self.height {
            for x in 0..self.width {
                self.set_pixel(x, y, self.bg);
            }
        }
        self.cursor_x = 0;
        self.cursor_y = 0;
    }

    fn set_pixel(&mut self, x: usize, y: usize, color: Color) {
        if x >= self.width || y >= self.height {
            return;
        }
        let offset = y * self.stride * self.bpp + x * self.bpp;
        match self.format {
            PixelFormat::Rgb => {
                self.buffer[offset] = color.r;
                self.buffer[offset + 1] = color.g;
                self.buffer[offset + 2] = color.b;
            }
            PixelFormat::Bgr => {
                self.buffer[offset] = color.b;
                self.buffer[offset + 1] = color.g;
                self.buffer[offset + 2] = color.r;
            }
            PixelFormat::U8 => {
                let gray = ((color.r as u32 + color.g as u32 + color.b as u32) / 3) as u8;
                self.buffer[offset] = gray;
            }
            _ => {}
        }
    }

    fn newline(&mut self) {
        self.cursor_x = 0;
        self.cursor_y += FONT_HEIGHT;
        if self.cursor_y + FONT_HEIGHT > self.height {
            self.scroll();
        }
    }

    fn scroll(&mut self) {
        let row_bytes = self.stride * self.bpp;
        let shift = FONT_HEIGHT * row_bytes;
        let total = self.height * row_bytes;

        for i in 0..(total - shift) {
            self.buffer[i] = self.buffer[i + shift];
        }
        for i in (total - shift)..total {
            self.buffer[i] = 0;
        }
        if self.bg.r != 0 || self.bg.g != 0 || self.bg.b != 0 {
            for y in (self.height - FONT_HEIGHT)..self.height {
                for x in 0..self.width {
                    self.set_pixel(x, y, self.bg);
                }
            }
        }
        self.cursor_y -= FONT_HEIGHT;
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

    fn write_char(&mut self, c: char) {
        match c {
            '\n' => self.newline(),
            _ => {
                if self.cursor_x + FONT_WIDTH > self.width {
                    self.newline();
                }
                self.draw_char(c);
                self.cursor_x += FONT_WIDTH;
            }
        }
    }

    fn draw_char(&mut self, c: char) {
        let bitmap = match get_raster(c, FontWeight::Regular, RasterHeight::Size16) {
            Some(b) => b,
            None => return,
        };
        let w = bitmap.width();
        let h = bitmap.height();
        let raster = bitmap.raster(); // &[&[u8]] — срез строк

        for y in 0..h {
            let row = raster[y];
            for x in 0..w {
                let alpha = row[x];
                if alpha == 0 {
                    continue;
                }
                let color = blend(self.fg, self.bg, alpha);
                self.set_pixel(self.cursor_x + x, self.cursor_y + y, color);
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
    let writer = Writer::new(
        buffer,
        info.width,
        info.height,
        info.stride,
        info.bytes_per_pixel,
        info.pixel_format,
    );
    *WRITER.lock() = Some(writer);
}

pub fn clear() {
    if let Some(w) = WRITER.lock().as_mut() {
        w.clear();
    }
}

pub fn backspace() {
    if let Some(w) = WRITER.lock().as_mut() {
        w.backspace();
    }
}

pub fn _print(args: fmt::Arguments) {
    use core::fmt::Write;
    let mut guard = WRITER.lock();
    if let Some(w) = guard.as_mut() {
        w.write_fmt(args).unwrap();
    }
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