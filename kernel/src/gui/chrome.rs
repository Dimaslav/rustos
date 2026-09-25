//! Мелкие графические примитивы: иконки, стрелки, layout.

use crate::framebuffer::{Color, Writer};
use super::state::{TITLE_H, Window};

pub fn draw_arrow_left(w: &mut Writer, x: usize, y: usize, color: Color) {
    for i in 0..7usize {
        let len = 7 - i;
        for j in 0..len { w.set_pixel(x + 3 + i, y + 4 + j, color); }
    }
    w.fill_rect(x + 8, y + 7, 8, 2, color);
}

pub fn draw_arrow_up(w: &mut Writer, x: usize, y: usize, color: Color) {
    for i in 0..7usize {
        let len = 7 - i;
        for j in 0..len { w.set_pixel(x + 4 + j, y + 3 + i, color); }
    }
    w.fill_rect(x + 7, y + 8, 2, 8, color);
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

pub fn draw_file_icon(w: &mut Writer, x: usize, y: usize, is_dir: bool, bg: Color) {
    if is_dir {
        let folder = Color { r: 240, g: 190, b: 80 };
        w.fill_rect(x + 1, y + 4, 3, 3, folder);
        w.fill_rect(x, y + 6, 12, 9, folder);
        w.fill_rect(x, y + 6, 12, 1, Color { r: 200, g: 150, b: 50 });
    } else {
        let paper = Color { r: 230, g: 235, b: 245 };
        let edge = Color { r: 140, g: 150, b: 170 };
        w.fill_rect(x + 1, y + 2, 10, 14, paper);
        w.fill_rect(x + 8, y + 2, 3, 3, bg);
        w.fill_rect(x + 1, y + 2, 10, 1, edge);
        w.fill_rect(x + 1, y + 15, 10, 1, edge);
        w.fill_rect(x + 3, y + 6, 6, 1, edge);
        w.fill_rect(x + 3, y + 9, 6, 1, edge);
        w.fill_rect(x + 3, y + 12, 4, 1, edge);
    }
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