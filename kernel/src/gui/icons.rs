//! Векторные иконки приложений (outline-стиль).
//!
//! Не требуют файлов и bitmap-массивов — рисуются примитивами.
//! Все линии толщиной 2 px при `size >= 24`, иначе 1 px.

use crate::framebuffer::{Color, Writer};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum IconKind {
    Folder,
    File,
    Notepad,
    Todo,
    Calc,
    Paint,
    Terminal,
}

/// Рисует иконку `kind` в квадрате `[x, y, size, size]` цветом `color`.
pub fn draw(kind: IconKind, w: &mut Writer, x: usize, y: usize, size: usize, color: Color) {
    let t = if size >= 24 { 2 } else { 1 };
    match kind {
        IconKind::Folder   => folder(w, x, y, size, color, t),
        IconKind::File     => file(w, x, y, size, color, t),
        IconKind::Notepad  => notepad(w, x, y, size, color, t),
        IconKind::Todo     => todo(w, x, y, size, color, t),
        IconKind::Calc     => calc(w, x, y, size, color, t),
        IconKind::Paint    => paint(w, x, y, size, color, t),
        IconKind::Terminal => terminal(w, x, y, size, color, t),
    }
}

/// Папка: узкий язычок сверху-слева + большое тело.
fn folder(w: &mut Writer, x: usize, y: usize, s: usize, c: Color, t: usize) {
    let body_y = y + s / 4;
    // Верхняя грань язычка
    w.fill_rect(x + t, y + t, s / 2 - t, t, c);
    // Левая грань от язычка до тела
    w.fill_rect(x + t, y + t, t, s / 4 - t, c);
    // Правая грань язычка
    w.fill_rect(x + s / 2, y + t, t, s / 4 - t, c);
    // Тело: верхняя грань
    w.fill_rect(x + t, body_y, s - 2 * t, t, c);
    // Нижняя грань
    w.fill_rect(x + t, y + s - 2 * t, s - 2 * t, t, c);
    // Левая грань тела
    w.fill_rect(x + t, body_y, t, s - body_y - t, c);
    // Правая грань тела
    w.fill_rect(x + s - 2 * t, body_y, t, s - body_y - t, c);
}

/// Документ: прямоугольник с загнутым верхним-правым углом.
fn file(w: &mut Writer, x: usize, y: usize, s: usize, c: Color, t: usize) {
    let fold = s / 4;
    // Верхняя грань до загиба
    w.fill_rect(x + t, y + t, s - fold - t, t, c);
    // Верхняя грань после загиба
    w.fill_rect(x + s - 2 * t, y + fold, t, t, c);
    // Правый край загиба (короткая линия вниз от верхней грани)
    w.fill_rect(x + s - fold, y + t, t, fold - t, c);
    // Левая грань
    w.fill_rect(x + t, y + t, t, s - 2 * t, c);
    // Правая грань ниже загиба
    w.fill_rect(x + s - 2 * t, y + fold, t, s - fold - t, c);
    // Нижняя грань
    w.fill_rect(x + t, y + s - 2 * t, s - 2 * t, t, c);
}

/// Блокнот: файл + 3 горизонтальные линии.
fn notepad(w: &mut Writer, x: usize, y: usize, s: usize, c: Color, t: usize) {
    file(w, x, y, s, c, t);
    let lx = x + s / 4;
    let lw = s / 2;
    for i in 0..3 {
        let ly = y + s / 2 + i * (s / 8);
        w.fill_rect(lx, ly, lw, t, c);
    }
}

/// Задача: чек-бокс + галочка + 3 линии.
fn todo(w: &mut Writer, x: usize, y: usize, s: usize, c: Color, t: usize) {
    // Чек-бокс
    let bx = x + t;
    let by = y + t;
    let bs = s / 3;
    w.draw_border(bx, by, bs, bs, t, c);
    // Галочка — две диагонали
    let mid_x = bx + bs / 2;
    let mid_y = by + bs / 2;
    for i in 0..(bs / 3) {
        w.set_pixel(mid_x - (bs / 4) + i, mid_y + i, c);
    }
    for i in 0..(bs / 2) {
        w.set_pixel(mid_x + i, mid_y + (bs / 4) - i - 1, c);
    }
    // 3 линии текста справа
    let lx = x + s / 2;
    let lw = s / 3;
    for i in 0..3 {
        let ly = y + t + i * (s / 4);
        w.fill_rect(lx, ly, lw, t, c);
    }
}

/// Калькулятор: корпус, дисплей, сетка кнопок.
fn calc(w: &mut Writer, x: usize, y: usize, s: usize, c: Color, t: usize) {
    // Корпус
    w.draw_border(x + t, y + t, s - 2 * t, s - 2 * t, t, c);
    // Дисплей
    let dy = y + 3 * t + 1;
    let dh = s / 5;
    w.fill_rect(x + 2 * t, dy, s - 4 * t, t, c);
    w.fill_rect(x + 2 * t, dy + dh, s - 4 * t, t, c);
    w.fill_rect(x + 2 * t, dy, t, dh + t, c);
    w.fill_rect(x + s - 3 * t, dy, t, dh + t, c);
    // Сетка кнопок
    let grid_y = dy + dh + 3 * t;
    let grid_x = x + 2 * t;
    let cell_w = (s - 4 * t) / 3;
    let cell_h = (s - (grid_y - y) - 2 * t) / 3;
    for r in 0..3 {
        for col in 0..3 {
            let kx = grid_x + col * cell_w + 1;
            let ky = grid_y + r * cell_h + 1;
            let kw = cell_w.saturating_sub(2 * t);
            let kh = cell_h.saturating_sub(2 * t);
            if kw > 0 && kh > 0 {
                w.fill_rect(kx, ky, kw, t, c);
            }
        }
    }
}

/// Paint: кисть (ручка + щетина) + капля.
fn paint(w: &mut Writer, x: usize, y: usize, s: usize, c: Color, t: usize) {
    // Ручка — диагональ
    let steps = s / 2;
    for i in 0..steps {
        let px = x + 2 * t + i;
        let py = y + 2 * t + i;
        w.fill_rect(px, py, t, t, c);
    }
    // Щетина — расширяющаяся полоска
    let bx = x + s / 2;
    let by = y + s / 2;
    let bs = s / 3;
    for i in 0..bs {
        w.fill_rect(bx + i, by + i, bs - i, t, c);
    }
    // Капля краски
    w.fill_rect(x + s - 5 * t, y + s - 3 * t, 2 * t, 2 * t, c);
}

/// Терминал: корпус, символ `>` + курсор.
fn terminal(w: &mut Writer, x: usize, y: usize, s: usize, c: Color, t: usize) {
    w.draw_border(x + t, y + t, s - 2 * t, s - 2 * t, t, c);
    // Символ `>`
    let px = x + 3 * t;
    let py = y + 3 * t;
    let len = s / 4;
    for i in 0..len {
        w.set_pixel(px + i, py + i, c);
        if py + 2 * len - i - 1 < y + s - 2 * t {
            w.set_pixel(px + i, py + 2 * len - i - 1, c);
        }
    }
    // Курсор
    let cx = px + len + t;
    let cy = py + 2 * len - t;
    if cy + t < y + s - 2 * t {
        w.fill_rect(cx, cy, len, t, c);
    }
}