use crate::framebuffer::{Color, Writer, FONT_HEIGHT};
use crate::gui::icons::{self, IconKind};
use crate::gui::theme;

pub const BUTTON_H: usize = 30;
pub const FIELD_H: usize = 26;
pub const ROW_H: usize = 20;

pub const ACCENT: Color = Color { r: 90, g: 140, b: 255 };
pub const SURFACE: Color = Color { r: 240, g: 242, b: 248 };
pub const SURFACE_DARK: Color = Color { r: 32, g: 34, b: 40 };
pub const TEXT_DARK: Color = Color { r: 24, g: 26, b: 32 };
pub const TEXT_LIGHT: Color = Color { r: 235, g: 238, b: 245 };
pub const MUTED: Color = Color { r: 140, g: 145, b: 160 };
pub const SHADOW: Color = Color { r: 0, g: 0, b: 0 };

pub fn button_modern(
    w: &mut Writer,
    x: usize, y: usize, bw: usize, bh: usize,
    label: &str, bg: Color, fg: Color, hover: bool,
) {
    let bg = if hover { theme::palette().accent_hover } else { bg };
    w.fill_round_rect(x + 1, y + 2, bw, bh, 6, theme::palette().shadow);
    w.fill_round_rect(x, y, bw, bh, 6, bg);
    let tw = Writer::text_width(label);
    let tx = x + (bw.saturating_sub(tw)) / 2;
    let ty = y + (bh.saturating_sub(FONT_HEIGHT)) / 2;
    w.draw_text_at(tx, ty, label, fg, bg);
}

pub fn text_field_modern(
    w: &mut Writer,
    x: usize, y: usize, fw: usize, fh: usize,
    text: &str, focused: bool,
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

pub fn list_box_modern(
    w: &mut Writer,
    x: usize, y: usize, lw: usize, lh: usize,
    items: &[alloc::string::String], selected: Option<usize>,
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
        let (fg, bg) = if sel { (Color::WHITE, p.accent) } else { (p.text, p.field_bg) };
        w.draw_text_at(x + 12, iy + 1, item, fg, bg);
    }
}

pub fn desktop_icon_modern(
    w: &mut Writer,
    x: usize, y: usize,
    label: &str, icon_kind: IconKind, color: Color, selected: bool,
) {
    let p = theme::palette();
    let size = 44usize;
    w.fill_round_rect(x + 2, y + 3, size, size, 10, p.shadow);
    w.fill_round_rect(x, y, size, size, 10, color);
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
    px >= x as i32 && py >= y as i32
        && (px as usize) < x + w
        && (py as usize) < y + h
}

// ============================================================
//                       НОВЫЕ ВИДЖЕТЫ
// ============================================================

pub fn checkbox(
    w: &mut Writer,
    x: usize, y: usize,
    label: &str, checked: bool, hover: bool,
) -> usize {
    let p = theme::palette();
    let size = 18usize;
    let box_bg = if hover { p.accent_hover } else { p.field_bg };
    w.fill_round_rect(x, y, size, size, 4, box_bg);
    for i in 0..size {
        w.set_pixel(x, y + i, p.border);
        w.set_pixel(x + size - 1, y + i, p.border);
        w.set_pixel(x + i, y, p.border);
        w.set_pixel(x + i, y + size - 1, p.border);
    }
    if checked {
        let cx = x + size / 2;
        let cy = y + size / 2;
        for i in 0..5 {
            w.set_pixel(cx - 5 + i, cy + i - 1, p.accent);
            w.set_pixel(cx - 5 + i, cy + i, p.accent);
        }
        for i in 0..8 {
            w.set_pixel(cx - 1 + i, cy - 1 - i + 4, p.accent);
            w.set_pixel(cx - 1 + i, cy - i + 4, p.accent);
        }
    }
    let label_w = Writer::text_width(label);
    w.draw_text_at(x + size + 8, y + (size - FONT_HEIGHT) / 2, label, p.text, p.window_bg);
    size + 8 + label_w
}

pub fn slider_h(
    w: &mut Writer,
    x: usize, y: usize, width: usize,
    value: u32, max: u32, hover: bool,
) -> usize {
    let p = theme::palette();
    let track_h = 4usize;
    let track_y = y + 8;
    w.fill_round_rect(x, track_y, width, track_h, 2, p.window_bg_alt);
    let frac = if max > 0 { value as f32 / max as f32 } else { 0.0 };
    let fill_w = (width as f32 * frac) as usize;
    w.fill_round_rect(x, track_y, fill_w, track_h, 2, p.accent);

    let handle_r = if hover { 9 } else { 7 };
    let hx = x + fill_w;
    let hy = y + 10;
    let hx0 = hx.saturating_sub(handle_r);
    let hy0 = hy.saturating_sub(handle_r);
    w.fill_round_rect_aa(hx0, hy0, handle_r * 2, handle_r * 2, handle_r, p.accent);
    hx
}

pub fn progressbar(
    w: &mut Writer,
    x: usize, y: usize, width: usize, height: usize,
    value: u32,
) {
    let p = theme::palette();
    w.fill_round_rect(x, y, width, height, height / 2, p.window_bg_alt);
    let frac = (value.min(100) as f32) / 100.0;
    let fill_w = (width as f32 * frac) as usize;
    if fill_w > 0 {
        w.fill_round_rect(x, y, fill_w.max(height), height, height / 2, p.accent);
    }
}

pub fn tabs(
    w: &mut Writer,
    x: usize, y: usize,
    labels: &[&str], active: usize,
    hover_idx: Option<usize>,
) -> usize {
    let p = theme::palette();
    let pad = 14usize;
    let height = 28usize;
    let mut cx = x;
    for (i, label) in labels.iter().enumerate() {
        let lw = Writer::text_width(label) + pad * 2;
        let is_active = i == active;
        let is_hover = hover_idx == Some(i);

        let bg = if is_active {
            p.accent
        } else if is_hover {
            p.accent_hover
        } else {
            p.window_bg_alt
        };

        w.fill_round_rect(cx, y, lw, height, 6, bg);

        let tw = Writer::text_width(label);
        w.draw_text_at(cx + (lw - tw) / 2, y + (height - FONT_HEIGHT) / 2, label, p.text, bg);

        if is_active {
            w.fill_rect(cx + 6, y + height - 2, lw - 12, 2, p.accent_hover);
        }

        cx += lw + 4;
    }
    cx - x
}

pub fn tooltip(w: &mut Writer, x: usize, y: usize, text: &str) {
    let p = theme::palette();
    let pad_x = 10usize;
    let pad_y = 6usize;
    let tw = Writer::text_width(text);
    let ww = tw + pad_x * 2;
    let wh = FONT_HEIGHT + pad_y * 2;

    w.fill_round_rect_aa(x + 2, y + 3, ww, wh, 6, p.shadow);
    w.fill_round_rect_aa(x, y, ww, wh, 6, Color { r: 30, g: 32, b: 40 });
    w.draw_text_at(x + pad_x, y + pad_y, text, Color::WHITE, Color { r: 30, g: 32, b: 40 });
}

/// Spinner без тригонометрии: 12 фиксированных позиций по кругу.
///
/// Таблица единичных векторов для 12 позиций (x, y) в диапазоне [-1, 1].
/// Для радиуса R: px = cx + (R * table_x / 256), py = cy + (R * table_y / 256).
pub fn spinner(w: &mut Writer, cx: usize, cy: usize, radius: usize, phase: u8) {
    const N: usize = 12;
    // x-компоненты, домноженные на 256.
    const TABLE_X: [i32; N] = [
        256, 222, 128, 0, -128, -222, -256, -222, -128, 0, 128, 222,
    ];
    // y-компоненты, домноженные на 256.
    const TABLE_Y: [i32; N] = [
        0, 128, 222, 256, 222, 128, 0, -128, -222, -256, -222, -128,
    ];

    let p = theme::palette();
    let r = radius as i32;
    for i in 0..N {
        let px = cx as i32 + (r * TABLE_X[i]) / 256;
        let py = cy as i32 + (r * TABLE_Y[i]) / 256;
        if px < 0 || py < 0 { continue; }
        let diff = ((i as i32 - phase as i32).rem_euclid(N as i32)) as u32;
        let alpha = (255 - diff * (200 / N as u32)) as u8;
        w.blend_pixel(px as usize, py as usize, p.accent, alpha);
    }
}

pub fn scrollbar_v(
    w: &mut Writer,
    x: usize, y: usize, width: usize, height: usize,
    scroll: usize, max_scroll: usize, total: usize, visible: usize,
) {
    let p = theme::palette();
    w.fill_round_rect(x, y, width, height, 4, p.window_bg_alt);

    if max_scroll == 0 || total <= visible {
        return;
    }

    let thumb_h = ((height as f32) * (visible as f32 / total as f32)) as usize;
    let thumb_h = thumb_h.max(20).min(height);
    let frac = if max_scroll > 0 {
        scroll as f32 / max_scroll as f32
    } else {
        0.0
    };
    let thumb_y = y + ((height - thumb_h) as f32 * frac) as usize;

    w.fill_round_rect(x + 1, thumb_y, width - 2, thumb_h, 4, p.text_muted);
}

pub fn badge(w: &mut Writer, cx: usize, cy: usize, r: usize, count: u32) {
    let p = theme::palette();
    let x = cx.saturating_sub(r);
    let y = cy.saturating_sub(r);
    w.fill_round_rect_aa(x, y, r * 2, r * 2, r, p.danger);
    if count > 0 && count < 10 {
        let s = alloc::format!("{}", count);
        let tw = Writer::text_width(&s);
        w.draw_text_at(
            cx.saturating_sub(tw / 2),
            cy.saturating_sub(FONT_HEIGHT / 2),
            &s,
            Color::WHITE,
            p.danger,
        );
    }
}