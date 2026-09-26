use crate::framebuffer::{Color, Writer, FONT_HEIGHT};
use crate::gui::icons::{self, IconKind};
use crate::gui::theme;

pub const BUTTON_H: usize = 30;
pub const FIELD_H: usize = 26;
pub const ROW_H: usize = 20;

pub const ACCENT: Color = Color { r: 90, g: 140, b: 255 };
pub const ACCENT_HOVER: Color = Color { r: 120, g: 165, b: 255 };
pub const SURFACE: Color = Color { r: 240, g: 242, b: 248 };
pub const SURFACE_DARK: Color = Color { r: 32, g: 34, b: 40 };
pub const SURFACE_DARK_2: Color = Color { r: 45, g: 48, b: 56 };
pub const TEXT_DARK: Color = Color { r: 24, g: 26, b: 32 };
pub const TEXT_LIGHT: Color = Color { r: 235, g: 238, b: 245 };
pub const MUTED: Color = Color { r: 140, g: 145, b: 160 };
pub const SHADOW: Color = Color { r: 0, g: 0, b: 0 };

/// Кнопка с тенью. Цвета берутся из аргументов, а не из темы — caller
/// сам решает, использовать ли accent, danger, window_bg_alt и т. д.
pub fn button_modern(
    w: &mut Writer,
    x: usize,
    y: usize,
    bw: usize,
    bh: usize,
    label: &str,
    bg: Color,
    fg: Color,
    hover: bool,
) {
    let bg = if hover { theme::palette().accent_hover } else { bg };
    w.fill_round_rect(x + 1, y + 2, bw, bh, 6, theme::palette().shadow);
    w.fill_round_rect(x, y, bw, bh, 6, bg);
    let tw = Writer::text_width(label);
    let tx = x + (bw.saturating_sub(tw)) / 2;
    let ty = y + (bh.saturating_sub(FONT_HEIGHT)) / 2;
    w.draw_text_at(tx, ty, label, fg, bg);
}

/// Поле ввода с рамкой и курсором в конце.
pub fn text_field_modern(
    w: &mut Writer,
    x: usize,
    y: usize,
    fw: usize,
    fh: usize,
    text: &str,
    focused: bool,
) {
    let p = theme::palette();
    let bg = p.field_bg;
    w.fill_round_rect(x, y, fw, fh, 4, bg);
    if focused {
        let border = p.accent;
        for i in 0..fh {
            w.set_pixel(x, y + i, border);
            w.set_pixel(x + fw - 1, y + i, border);
        }
        for i in 0..fw {
            w.set_pixel(x + i, y, border);
            w.set_pixel(x + i, y + fh - 1, border);
        }
    }

    let pad_x = 8;
    let pad_y = (fh.saturating_sub(FONT_HEIGHT)) / 2;
    let max_chars = (fw.saturating_sub(pad_x * 2)) / 9;
    let chars: alloc::vec::Vec<char> = text.chars().collect();
    let start = if chars.len() > max_chars { chars.len() - max_chars } else { 0 };
    let visible: alloc::string::String = chars[start..].iter().collect();
    w.draw_text_at(x + pad_x, y + pad_y, &visible, p.text, bg);

    if focused {
        let cx = x + pad_x + Writer::text_width(&visible);
        let cy = y + pad_y;
        w.fill_rect(cx, cy, 2, FONT_HEIGHT, p.accent);
    }
}

/// Простой список с выделением и прокруткой (визуально — только видимые строки).
pub fn list_box_modern(
    w: &mut Writer,
    x: usize,
    y: usize,
    lw: usize,
    lh: usize,
    items: &[alloc::string::String],
    selected: Option<usize>,
) {
    let p = theme::palette();
    w.fill_round_rect(x, y, lw, lh, 4, p.field_bg);
    let pad = 6;
    let max_rows = (lh.saturating_sub(pad * 2)) / ROW_H;
    for (i, item) in items.iter().take(max_rows).enumerate() {
        let iy = y + pad + i * ROW_H;
        let sel = selected == Some(i);
        if sel {
            w.fill_round_rect(x + 4, iy, lw - 8, ROW_H - 2, 4, p.accent);
        }
        let (fg, bg) = if sel {
            (Color::WHITE, p.accent)
        } else {
            (p.text, p.field_bg)
        };
        w.draw_text_at(x + 12, iy + 1, item, fg, bg);
    }
}

/// Иконка + подпись + подсветка выделения. Всё в цветах `theme::palette()`.
pub fn desktop_icon_modern(
    w: &mut Writer,
    x: usize,
    y: usize,
    label: &str,
    icon_kind: IconKind,
    color: Color,
    selected: bool,
) {
    let p = theme::palette();
    let size = 44usize;
    w.fill_round_rect(x + 2, y + 3, size, size, 10, p.shadow);
    w.fill_round_rect(x, y, size, size, 10, color);

    // Иконка по центру квадрата 44×44, размер 28×28.
    let icon_size = 28usize;
    let icon_x = x + (size - icon_size) / 2;
    let icon_y = y + (size - icon_size) / 2;
    icons::draw(icon_kind, w, icon_x, icon_y, icon_size, Color::WHITE);

    let tw = Writer::text_width(label);
    let tx = x + (size.saturating_sub(tw)) / 2;
    let ty = y + size + 6;

    if selected {
        let bg = p.accent;
        w.fill_round_rect(tx.saturating_sub(4), ty.saturating_sub(2), tw + 8, FONT_HEIGHT + 4, 4, bg);
        w.draw_text_at(tx, ty, label, Color::WHITE, bg);
    } else {
        let bg = p.shadow;
        w.fill_round_rect(tx.saturating_sub(4), ty.saturating_sub(2), tw + 8, FONT_HEIGHT + 4, 4, bg);
        w.draw_text_at(tx, ty, label, p.text, bg);
    }
}

pub fn hit(px: i32, py: i32, x: usize, y: usize, w: usize, h: usize) -> bool {
    px >= x as i32 && py >= y as i32 && (px as usize) < x + w && (py as usize) < y + h
}