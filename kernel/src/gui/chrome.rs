//! Мелкие графические примитивы: иконки, стрелки, layout.

use alloc::string::ToString;
use crate::framebuffer::{Color, Writer};
use crate::gui::icons::{self, IconKind};
use super::state::{TITLE_H, Window};

pub fn draw_arrow_left(w: &mut Writer, x: usize, y: usize, color: Color) {
    for i in 0..10usize {
        let len = 10 - i;
        for j in 0..len {
            w.set_pixel(x + 2 + i, y + 7 + j - len / 2, color);
        }
    }
    w.fill_rect(x + 12, y + 6, 8, 2, color);
}

pub fn draw_arrow_up(w: &mut Writer, x: usize, y: usize, color: Color) {
    for i in 0..10usize {
        let len = 10 - i;
        for j in 0..len {
            w.set_pixel(x + 7 + j - len / 2, y + 2 + i, color);
        }
    }
    w.fill_rect(x + 6, y + 12, 2, 8, color);
}

pub fn draw_plus_icon(w: &mut Writer, x: usize, y: usize, color: Color) {
    w.fill_rect(x + 6, y + 3, 2, 12, color);
    w.fill_rect(x + 1, y + 8, 12, 2, color);
}

pub fn draw_trash_icon(w: &mut Writer, x: usize, y: usize, color: Color) {
    w.fill_rect(x + 2, y + 3, 10, 2, color);
    w.fill_rect(x + 5, y + 1, 4, 2, color);
    w.fill_rect(x + 3, y + 5, 8, 8, color);
}

pub fn draw_pencil_icon(w: &mut Writer, x: usize, y: usize, color: Color) {
    for i in 0..10usize {
        w.fill_rect(x + 3 + i, y + 11 - i, 2, 2, color);
    }
    w.set_pixel(x + 3, y + 13, color);
    w.set_pixel(x + 2, y + 14, color);
}

/// Иконка файла по расширению. Директория — жёлтая папка.
/// `.txt` — Notepad, `.png`/`.bmp` — Paint, `.exe`/`.elf` — Terminal,
/// всё остальное — generic File.
pub fn draw_file_icon(w: &mut Writer, x: usize, y: usize, name: &str, is_dir: bool, size: usize) {
    if is_dir {
        icons::draw(IconKind::Folder, w, x, y, size, Color { r: 240, g: 190, b: 80 });
        return;
    }
    let lower = name.to_ascii_lowercase();
    let (kind, color) = if lower.ends_with(".txt") {
        (IconKind::Notepad, Color { r: 100, g: 150, b: 255 })
    } else if lower.ends_with(".png") || lower.ends_with(".bmp") || lower.ends_with(".jpg") {
        (IconKind::Paint, Color { r: 220, g: 90, b: 180 })
    } else if lower.ends_with(".exe") || lower.ends_with(".elf") {
        (IconKind::Terminal, Color { r: 90, g: 200, b: 120 })
    } else {
        (IconKind::File, Color { r: 230, g: 235, b: 245 })
    };
    icons::draw(kind, w, x, y, size, color);
}

pub fn sidebar_icon(kind: SidebarIcon) -> (IconKind, Color) {
    match kind {
        SidebarIcon::Home       => (IconKind::Folder,   Color { r: 255, g: 195, b: 70 }),
        SidebarIcon::Desktop    => (IconKind::Folder,   Color { r: 90,  g: 160, b: 255 }),
        SidebarIcon::Documents  => (IconKind::Folder,   Color { r: 220, g: 165, b: 90 }),
        SidebarIcon::Downloads  => (IconKind::Folder,   Color { r: 80,  g: 200, b: 120 }),
        SidebarIcon::System     => (IconKind::Terminal, Color { r: 160, g: 165, b: 180 }),
        SidebarIcon::Disk       => (IconKind::Terminal, Color { r: 100, g: 180, b: 255 }),
    }
}

#[derive(Clone, Copy)]
pub enum SidebarIcon {
    Home,
    Desktop,
    Documents,
    Downloads,
    System,
    Disk,
}

pub struct CalcLayout {
    pub display_x: usize,
    pub display_y: usize,
    pub display_w: usize,
    pub display_h: usize,
    pub grid_x: usize,
    pub grid_y: usize,
    pub step: usize,
}

pub fn calc_layout(win: &Window) -> CalcLayout {
    let pad = 16;
    let x = win.x.max(0) as usize;
    let y = win.y.max(0) as usize;
    let dx = x + pad;
    let dy = y + TITLE_H + pad;
    let dw = win.w.saturating_sub(pad * 2);
    let dh = 42usize;
    let bx = x + pad;
    let by = dy + dh + 62;
    CalcLayout {
        display_x: dx,
        display_y: dy,
        display_w: dw,
        display_h: dh,
        grid_x: bx,
        grid_y: by,
        step: 70,
    }
}