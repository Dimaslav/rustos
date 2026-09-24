use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::arch::asm;

use crate::framebuffer::{self, Color, Writer, FONT_HEIGHT};
use crate::interrupts::{ticks, uptime_secs};
use crate::keyboard::{self, Key};
use crate::mouse::{self, MouseButton, MouseEvent};
use crate::sound;
use crate::widgets::{
    self, ACCENT, BUTTON_H, FIELD_H, MUTED, ROW_H, SURFACE_DARK, SURFACE_DARK_2, TEXT_LIGHT,
};

const TITLE_H: usize = 30;
const TASKBAR_H: usize = 42;
const CLOSE_BTN_W: usize = 26;
const MIN_BTN_W: usize = 26;

const WALLPAPER_TOP: Color = Color { r: 18, g: 22, b: 34 };
const WALLPAPER_BOTTOM: Color = Color { r: 40, g: 30, b: 60 };

const ICON_X: usize = 32;
const ICON_Y: usize = 32;
const ICON_STEP: usize = 110;

#[derive(Clone)]
enum App {
    Notepad { text: String },
    Todo { items: Vec<String>, selected: Option<usize>, input: String },
    Calculator { display: String, a: f64, op: char, fresh: bool },
    Paint { canvas: Vec<u8>, w: usize, h: usize, last: Option<(usize, usize)> },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AppKind { Notepad, Todo, Calculator, Paint }

struct Window {
    x: i32,
    y: i32,
    w: usize,
    h: usize,
    title: String,
    content: App,
    minimized: bool,
}

struct Drag { idx: usize, ox: i32, oy: i32 }

struct Wm {
    windows: Vec<Window>,
    active: usize,
    drag: Option<Drag>,
    painting: Option<usize>,
    start_pressed: bool,
    last_click: Option<(u64, AppKind)>,
    hover: (i32, i32),
    dirty: bool,
}

pub fn run() -> ! {
    let (sw, sh) = framebuffer::with_writer(|w| (w.width, w.height));
    mouse::set_screen(sw, sh);

    let mut wm = Wm::new();
    let mut last_pos = mouse::position();
    wm.hover = last_pos;
    wm.dirty = true;
    let mut last_clock_tick = ticks();

    loop {
        sound::tick();

        // Coalesced Move: читаем только последнюю позицию. Не плодим события.
        let pos = mouse::position();
        if pos != last_pos {
            last_pos = pos;
            wm.on_move(pos.0, pos.1);
        }

        // Button events — из очереди (edge).
        while let Some(ev) = mouse::pop_event() {
            match ev {
                MouseEvent::ButtonDown(MouseButton::Left) => wm.on_button_down(pos.0, pos.1),
                MouseEvent::ButtonUp(MouseButton::Left) => wm.on_button_up(pos.0, pos.1),
                _ => {}
            }
        }

        while let Some(k) = keyboard::pop() {
            let (_, _, alt) = keyboard::modifiers();
            wm.on_key(k, alt);
        }

        // Часы — раз в 18 тиков (~1 сек), с хранением последнего тика.
        let now = ticks();
        if now.saturating_sub(last_clock_tick) >= 18 {
            last_clock_tick = now;
            wm.dirty = true;
        }

        if wm.dirty {
            framebuffer::with_writer(|w| wm.draw(w));
            framebuffer::flush();
            wm.dirty = false;
        }

        unsafe { asm!("hlt"); }
    }
}

impl Wm {
    fn new() -> Self {
        let mut wm = Wm {
            windows: Vec::new(),
            active: 0,
            drag: None,
            painting: None,
            start_pressed: false,
            last_click: None,
            hover: mouse::position(),
            dirty: true,
        };
        wm.windows.push(Window {
            x: 420,
            y: 200,
            w: 480,
            h: 220,
            title: "Welcome".to_string(),
            content: App::Notepad {
                text: "Rust OS v0.5\n\nDouble-click a desktop icon to open an app.\nAlt+Tab switches windows.\nAlt+F4 closes the active window.\n\nDrag the title bar to move a window.".to_string(),
            },
            minimized: false,
        });
        wm.active = 0;
        wm
    }

    fn icons() -> [(AppKind, &'static str, char, Color); 4] {
        [
            (AppKind::Notepad,    "Notepad",    'N', Color { r: 100, g: 150, b: 255 }),
            (AppKind::Todo,       "Tasks",      'T', Color { r: 80,  g: 200, b: 120 }),
            (AppKind::Calculator, "Calculator", 'C', Color { r: 240, g: 150, b: 60  }),
            (AppKind::Paint,      "Paint",      'P', Color { r: 220, g: 90,  b: 180 }),
        ]
    }

    fn icon_rect(i: usize) -> (usize, usize, usize, usize) {
        let col = i / 4;
        let row = i % 4;
        (ICON_X + col * ICON_STEP, ICON_Y + row * 90, 44, 66)
    }

    fn focus_window(&mut self, idx: usize) {
        if idx >= self.windows.len() { return; }
        if idx != self.windows.len() - 1 {
            let win = self.windows.remove(idx);
            self.windows.push(win);
        }
        self.active = self.windows.len() - 1;
        self.windows[self.active].minimized = false;
        self.dirty = true;
    }

    fn close_active(&mut self) {
        if self.windows.is_empty() { return; }
        self.windows.remove(self.active);
        if self.windows.is_empty() {
            self.active = 0;
        } else if self.active >= self.windows.len() {
            self.active = self.windows.len() - 1;
        }
        self.dirty = true;
    }

    fn open_app(&mut self, kind: AppKind) {
        for (i, w) in self.windows.iter().enumerate() {
            let m = matches!(
                (&w.content, kind),
                (App::Notepad { .. }, AppKind::Notepad)
                    | (App::Todo { .. }, AppKind::Todo)
                    | (App::Calculator { .. }, AppKind::Calculator)
                    | (App::Paint { .. }, AppKind::Paint)
            );
            if m {
                self.focus_window(i);
                sound::open();
                return;
            }
        }

        let (title, content) = match kind {
            AppKind::Notepad => ("Notepad", App::Notepad { text: String::new() }),
            AppKind::Todo => (
                "Tasks",
                App::Todo {
                    items: vec!["Write UI".to_string(), "Add sound".to_string()],
                    selected: None,
                    input: String::new(),
                },
            ),
            AppKind::Calculator => (
                "Calculator",
                App::Calculator { display: "0".to_string(), a: 0.0, op: ' ', fresh: true },
            ),
            AppKind::Paint => (
                "Paint",
                App::Paint { canvas: vec![255u8; 320 * 200 * 3], w: 320, h: 200, last: None },
            ),
        };

        let off = self.windows.len() as i32 * 28;
        self.windows.push(Window {
            x: 140 + off,
            y: 90 + off,
            w: 500,
            h: 340,
            title: title.to_string(),
            content,
            minimized: false,
        });
        self.active = self.windows.len() - 1;
        self.dirty = true;
        sound::open();
    }

    // ---------- Mouse ----------

    fn on_move(&mut self, x: i32, y: i32) {
        self.hover = (x, y);
        if let Some(d) = &self.drag {
            let (idx, ox, oy) = (d.idx, d.ox, d.oy);
            if let Some(w) = self.windows.get_mut(idx) {
                w.x = (x - ox).max(0);
                w.y = (y - oy).max(0);
            }
            self.dirty = true;
        }
        if let Some(idx) = self.painting {
            self.paint_at(idx, x, y);
            self.dirty = true;
        }
    }

    fn on_button_down(&mut self, x: i32, y: i32) {
        self.dirty = true;

        let (_, sh) = mouse::screen_size();
        let taskbar_y = (sh as usize).saturating_sub(TASKBAR_H);

        if (y as usize) >= taskbar_y {
            if x < 78 {
                self.start_pressed = !self.start_pressed;
                return;
            }
            let mut bx = 86;
            for (idx, w) in self.windows.iter().enumerate() {
                let bw = Writer::text_width(&w.title) + 24;
                if (x as usize) >= bx && (x as usize) < bx + bw {
                    if idx == self.active {
                        self.windows[idx].minimized = !self.windows[idx].minimized;
                    } else {
                        self.focus_window(idx);
                    }
                    return;
                }
                bx += bw + 4;
            }
            return;
        }
        self.start_pressed = false;

        let icons = Self::icons();
        for (i, (kind, _, _, _)) in icons.iter().enumerate() {
            let (ix, iy, iw, ih) = Self::icon_rect(i);
            if widgets::hit(x, y, ix, iy, iw, ih) {
                let now = ticks();
                let dbl = matches!(
                    self.last_click,
                    Some((t, k)) if k == *kind && now.saturating_sub(t) < 30
                );
                if dbl {
                    self.open_app(*kind);
                    self.last_click = None;
                } else {
                    self.last_click = Some((now, *kind));
                }
                return;
            }
        }

        let mut hit_idx = None;
        for idx in (0..self.windows.len()).rev() {
            if self.windows[idx].minimized { continue; }
            let w = &self.windows[idx];
            if widgets::hit(x, y, w.x as usize, w.y as usize, w.w, w.h) {
                hit_idx = Some(idx);
                break;
            }
        }
        if let Some(idx) = hit_idx {
            self.focus_window(idx);
            let (wx, wy, ww) = {
                let w = &self.windows[self.active];
                (w.x, w.y, w.w)
            };
            let rel_x = (x - wx) as usize;
            let rel_y = (y - wy) as usize;

            if rel_y < TITLE_H {
                let cb_x = ww.saturating_sub(8 + CLOSE_BTN_W);
                let mb_x = cb_x.saturating_sub(6 + MIN_BTN_W);
                if rel_x >= cb_x {
                    self.close_active();
                } else if rel_x >= mb_x {
                    let i = self.active;
                    self.windows[i].minimized = true;
                } else {
                    self.drag = Some(Drag {
                        idx: self.active,
                        ox: x - wx,
                        oy: y - wy,
                    });
                }
            } else {
                self.handle_content_click(x, y);
            }
        }
    }

    fn on_button_up(&mut self, _x: i32, _y: i32) {
        self.drag = None;
        self.painting = None;
        self.dirty = true;
    }

    // ---------- Keyboard ----------

    fn on_key(&mut self, k: Key, alt: bool) {
        if alt && k == Key::Tab && !self.windows.is_empty() {
            let next = (self.active + 1) % self.windows.len();
            self.focus_window(next);
            return;
        }
        if alt && k == Key::F(4) && !self.windows.is_empty() {
            self.close_active();
            return;
        }
        if k == Key::Escape {
            self.start_pressed = false;
            self.dirty = true;
            return;
        }

        if self.windows.is_empty() { return; }
        let idx = self.active;
        let win = &mut self.windows[idx];
        win.minimized = false;
        self.dirty = true;

        match &mut win.content {
            App::Notepad { text } => match k {
                Key::Enter => text.push('\n'),
                Key::Backspace => { text.pop(); }
                Key::Tab => text.push_str("    "),
                Key::Char(c) => text.push(c as char),
                _ => {}
            },
            App::Todo { items, input, .. } => match k {
                Key::Enter => {
                    let s = input.trim();
                    if !s.is_empty() {
                        items.push(s.to_string());
                        input.clear();
                    }
                }
                Key::Backspace => { input.pop(); }
                Key::Char(c) => input.push(c as char),
                _ => {}
            },
            App::Calculator { .. } => match k {
                Key::Char(c) => self.calc_input(c as char),
                Key::Enter => self.calc_input('='),
                Key::Backspace => self.calc_input('C'),
                _ => {}
            },
            App::Paint { .. } => {}
        }
    }

    // ---------- Application logic ----------

    fn calc_input(&mut self, ch: char) {
        let win = &mut self.windows[self.active];
        let (display, a, op, fresh) = match &mut win.content {
            App::Calculator { display, a, op, fresh } => (display, a, op, fresh),
            _ => return,
        };
        match ch {
            '0'..='9' | '.' => {
                if *fresh { display.clear(); *fresh = false; }
                if ch == '.' && display.contains('.') { return; }
                display.push(ch);
            }
            '+' | '-' | '*' | '/' => {
                *a = display.parse::<f64>().unwrap_or(0.0);
                *op = ch;
                *fresh = true;
            }
            '=' => {
                let b = display.parse::<f64>().unwrap_or(0.0);
                let r = match *op {
                    '+' => *a + b,
                    '-' => *a - b,
                    '*' => *a * b,
                    '/' => if b != 0.0 { *a / b } else { f64::NAN },
                    _ => b,
                };
                let rounded = r as i64;
                *display = if (r - rounded as f64).abs() < 0.0001 {
                    format!("{}", rounded)
                } else {
                    format!("{:.4}", r)
                };
                *op = ' ';
                *fresh = true;
            }
            'C' => {
                display.clear();
                display.push('0');
                *a = 0.0;
                *op = ' ';
                *fresh = true;
            }
            _ => {}
        }
    }

    fn handle_content_click(&mut self, mx: i32, my: i32) {
        let active = self.active;
        let win = &mut self.windows[active];
        let wx = win.x.max(0) as usize;
        let wy = win.y.max(0) as usize;

        match &mut win.content {
            App::Notepad { .. } => {}

            App::Todo { items, selected, input } => {
                let pad = 16;
                let field_x = wx + pad;
                let field_y = wy + TITLE_H + pad;
                let field_w = win.w.saturating_sub(pad * 3 + 80);
                let add_btn_x = field_x + field_w + 8;
                let list_x = wx + pad;
                let list_y = field_y + FIELD_H + 12;
                let list_w = win.w.saturating_sub(pad * 2);
                let list_h = win.h.saturating_sub(TITLE_H + FIELD_H + 4 * pad + BUTTON_H);
                let remove_btn_y = list_y + list_h + 10;

                if widgets::hit(mx, my, add_btn_x, field_y, 72, FIELD_H) {
                    let s = input.trim();
                    if !s.is_empty() {
                        items.push(s.to_string());
                        input.clear();
                    }
                    return;
                }
                if widgets::hit(mx, my, list_x, remove_btn_y, 120, BUTTON_H) {
                    if let Some(sel) = selected.take() {
                        if sel < items.len() {
                            items.remove(sel);
                        }
                    }
                    return;
                }
                if widgets::hit(mx, my, list_x, list_y, list_w, list_h) {
                    let rel = (my as usize).saturating_sub(list_y + 6);
                    let row = rel / ROW_H;
                    *selected = if row < items.len() { Some(row) } else { None };
                }
            }

            App::Calculator { .. } => {
                let layout = calc_layout(win);
                let (bx, by) = (layout.grid_x, layout.grid_y);
                let bw = 64usize;
                let bh = 44usize;
                let step = layout.step;

                if widgets::hit(mx, my, bx, by.saturating_sub(50), bw, 40) {
                    self.calc_input('C');
                    return;
                }
                let labels = [
                    ["7", "8", "9", "/"],
                    ["4", "5", "6", "*"],
                    ["1", "2", "3", "-"],
                    ["0", ".", "=", "+"],
                ];
                for (r, row) in labels.iter().enumerate() {
                    for (c, label) in row.iter().enumerate() {
                        let kx = bx + c * step;
                        let ky = by + r * (bh + 6);
                        if widgets::hit(mx, my, kx, ky, bw, bh) {
                            self.calc_input(label.chars().next().unwrap());
                            return;
                        }
                    }
                }
            }

            App::Paint { canvas, w, h, last } => {
                let pad = 10;
                let cx = wx + pad;
                let cy = wy + TITLE_H + pad;
                let cw = *w;
                let ch = *h;

                let clear_y = cy + ch + 10;
                if widgets::hit(mx, my, cx, clear_y, 100, 30) {
                    for p in canvas.iter_mut() { *p = 255; }
                    return;
                }
                if widgets::hit(mx, my, cx, cy, cw, ch) {
                    let px = (mx as usize).saturating_sub(cx).min(cw - 1);
                    let py = (my as usize).saturating_sub(cy).min(ch - 1);
                    *last = Some((px, py));
                    self.painting = Some(active);
                    self.paint_at(active, mx, my);
                }
            }
        }
    }

    fn paint_at(&mut self, idx: usize, mx: i32, my: i32) {
        if idx >= self.windows.len() { return; }
        let win = &mut self.windows[idx];
        let wx = win.x.max(0) as usize;
        let wy = win.y.max(0) as usize;

        if let App::Paint { canvas, w, h, last } = &mut win.content {
            let pad = 10;
            let cx = wx + pad;
            let cy = wy + TITLE_H + pad;
            let px = (mx as usize).saturating_sub(cx);
            let py = (my as usize).saturating_sub(cy);
            if px >= *w || py >= *h { return; }
            let (px0, py0) = last.unwrap_or((px, py));
            let steps = ((px as i32 - px0 as i32).abs())
                .max((py as i32 - py0 as i32).abs())
                .max(1);
            for s in 0..=steps {
                let t = s as f64 / steps as f64;
                let xx = (px0 as f64 + (px as f64 - px0 as f64) * t) as usize;
                let yy = (py0 as f64 + (py as f64 - py0 as f64) * t) as usize;
                for dy in 0..3 {
                    for dx in 0..3 {
                        let nx = xx + dx;
                        let ny = yy + dy;
                        if nx < *w && ny < *h {
                            let off = (ny * *w + nx) * 3;
                            canvas[off] = 0;
                            canvas[off + 1] = 0;
                            canvas[off + 2] = 0;
                        }
                    }
                }
            }
            *last = Some((px, py));
        }
    }

    // ---------- Drawing ----------

    fn draw(&self, w: &mut Writer) {
        w.gradient_v(0, 0, w.width, w.height, WALLPAPER_TOP, WALLPAPER_BOTTOM);

        let icons = Self::icons();
        let selected_kind = self.last_click.map(|(_, k)| k);
        for (i, (kind, label, ch, color)) in icons.iter().enumerate() {
            let (ix, iy, _, _) = Self::icon_rect(i);
            let sel = selected_kind == Some(*kind);
            widgets::desktop_icon_modern(w, ix, iy, label, *ch, *color, sel);
        }

        for (idx, win) in self.windows.iter().enumerate() {
            if win.minimized { continue; }
            let active = idx == self.active;
            self.draw_window(w, win, active);
        }

        self.draw_taskbar(w);

        if self.start_pressed {
            self.draw_start_menu(w);
        }

        let (mx, my) = mouse::position();
        draw_cursor(w, mx, my);
    }

    fn draw_window(&self, w: &mut Writer, win: &Window, active: bool) {
        let x = win.x.max(0) as usize;
        let y = win.y.max(0) as usize;
        let ww = win.w;
        let wh = win.h;

        w.fill_round_rect(x + 4, y + 4, ww, wh, 10, Color { r: 8, g: 10, b: 14 });
        w.fill_round_rect(x, y, ww, wh, 10, SURFACE_DARK);

        let title_bg = if active { ACCENT } else { SURFACE_DARK_2 };
        w.fill_round_rect(x, y, ww, TITLE_H + 10, 10, title_bg);
        w.fill_rect(x, y + TITLE_H, ww, 10, SURFACE_DARK);
        w.draw_text_at(
            x + 14,
            y + (TITLE_H - FONT_HEIGHT) / 2,
            &win.title,
            TEXT_LIGHT,
            title_bg,
        );

        let cb_x = x + ww.saturating_sub(12 + CLOSE_BTN_W);
        let cb_y = y + (TITLE_H - 20) / 2;
        self.draw_close_btn(w, cb_x, cb_y);
        let mb_x = cb_x.saturating_sub(6 + MIN_BTN_W);
        self.draw_min_btn(w, mb_x, cb_y);

        match &win.content {
            App::Notepad { text } => self.draw_notepad(w, win, text),
            App::Todo { items, selected, input } => {
                self.draw_todo(w, win, items, *selected, input)
            }
            App::Calculator { display, .. } => self.draw_calc(w, win, display),
            App::Paint { canvas, w: cw, h: chh, .. } => {
                self.draw_paint(w, win, canvas, *cw, *chh)
            }
        }
    }

    fn draw_close_btn(&self, w: &mut Writer, x: usize, y: usize) {
        let hover = widgets::hit(self.hover.0, self.hover.1, x, y, CLOSE_BTN_W, 20);
        let bg = if hover {
            Color { r: 232, g: 68, b: 68 }
        } else {
            SURFACE_DARK_2
        };
        w.fill_round_rect(x, y, CLOSE_BTN_W, 20, 6, bg);
        let cx = x + CLOSE_BTN_W / 2;
        let cy = y + 10;
        w.fill_rect(cx - 3, cy - 1, 7, 2, TEXT_LIGHT);
        w.fill_rect(cx - 1, cy - 3, 2, 7, TEXT_LIGHT);
    }

    fn draw_min_btn(&self, w: &mut Writer, x: usize, y: usize) {
        let hover = widgets::hit(self.hover.0, self.hover.1, x, y, MIN_BTN_W, 20);
        let bg = if hover {
            Color { r: 80, g: 90, b: 110 }
        } else {
            SURFACE_DARK_2
        };
        w.fill_round_rect(x, y, MIN_BTN_W, 20, 6, bg);
        let cx = x + MIN_BTN_W / 2;
        let cy = y + 12;
        w.fill_rect(cx - 4, cy, 8, 2, TEXT_LIGHT);
    }

    fn draw_notepad(&self, w: &mut Writer, win: &Window, text: &str) {
        let x = win.x.max(0) as usize;
        let y = win.y.max(0) as usize;
        let pad = 14;
        let field_x = x + pad;
        let field_y = y + TITLE_H + 6;
        let field_w = win.w.saturating_sub(pad * 2);
        let field_h = win.h.saturating_sub(TITLE_H + pad + 8);
        w.fill_round_rect(field_x, field_y, field_w, field_h, 6, SURFACE_DARK_2);

        let px = field_x + 8;
        let py = field_y + 8;
        let max_chars = field_w.saturating_sub(16) / 9;
        let mut line = 0usize;
        let mut col = 0usize;
        for c in text.chars() {
            if c == '\n' {
                line += 1;
                col = 0;
                if py + (line + 1) * FONT_HEIGHT > field_y + field_h { break; }
                continue;
            }
            if col >= max_chars {
                line += 1;
                col = 0;
                if py + (line + 1) * FONT_HEIGHT > field_y + field_h { break; }
            }
            w.draw_text_at(
                px + col * 9,
                py + line * FONT_HEIGHT,
                &c.to_string(),
                TEXT_LIGHT,
                SURFACE_DARK_2,
            );
            col += 1;
        }
    }

    fn draw_todo(
        &self,
        w: &mut Writer,
        win: &Window,
        items: &[String],
        selected: Option<usize>,
        input: &str,
    ) {
        let x = win.x.max(0) as usize;
        let y = win.y.max(0) as usize;
        let pad = 16;
        let field_x = x + pad;
        let field_y = y + TITLE_H + pad;
        let field_w = win.w.saturating_sub(pad * 3 + 80);
        let add_btn_x = field_x + field_w + 8;

        widgets::text_field_modern(w, field_x, field_y, field_w, FIELD_H, input, true);
        widgets::button_modern(
            w, add_btn_x, field_y, 72, FIELD_H,
            "Add", ACCENT, Color::WHITE, false,
        );

        let list_x = x + pad;
        let list_y = field_y + FIELD_H + 12;
        let list_w = win.w.saturating_sub(pad * 2);
        let list_h = win.h.saturating_sub(TITLE_H + FIELD_H + 4 * pad + BUTTON_H);
        widgets::list_box_modern(w, list_x, list_y, list_w, list_h, items, selected);

        let remove_btn_y = list_y + list_h + 10;
        widgets::button_modern(
            w, list_x, remove_btn_y, 120, BUTTON_H,
            "Remove",
            Color { r: 180, g: 60, b: 60 },
            Color::WHITE,
            false,
        );
    }

    fn draw_calc(&self, w: &mut Writer, win: &Window, display: &str) {
        let layout = calc_layout(win);
        let (dx, dy, dw, dh) = (
            layout.display_x, layout.display_y,
            layout.display_w, layout.display_h,
        );
        let (bx, by, step) = (layout.grid_x, layout.grid_y, layout.step);

        w.fill_round_rect(dx, dy, dw, dh, 6, SURFACE_DARK_2);
        let tw = Writer::text_width(display);
        w.draw_text_at(
            dx + dw.saturating_sub(tw + 12),
            dy + (dh - FONT_HEIGHT) / 2,
            display,
            TEXT_LIGHT,
            SURFACE_DARK_2,
        );

        widgets::button_modern(
            w, bx, by.saturating_sub(50), 64, 40,
            "C",
            Color { r: 180, g: 60, b: 60 },
            Color::WHITE,
            false,
        );

        let labels = [
            ["7", "8", "9", "/"],
            ["4", "5", "6", "*"],
            ["1", "2", "3", "-"],
            ["0", ".", "=", "+"],
        ];
        let bw = 64usize;
        let bh = 44usize;
        for (r, row) in labels.iter().enumerate() {
            for (c, label) in row.iter().enumerate() {
                let is_op = matches!(*label, "/" | "*" | "-" | "+" | "=");
                let bg = if is_op { ACCENT } else { SURFACE_DARK_2 };
                widgets::button_modern(
                    w,
                    bx + c * step,
                    by + r * (bh + 6),
                    bw, bh,
                    label,
                    bg,
                    Color::WHITE,
                    false,
                );
            }
        }
    }

    fn draw_paint(&self, w: &mut Writer, win: &Window, canvas: &[u8], cw: usize, ch: usize) {
        let x = win.x.max(0) as usize;
        let y = win.y.max(0) as usize;
        let pad = 10;
        let cx = x + pad;
        let cy = y + TITLE_H + pad;

        for yy in 0..ch {
            for xx in 0..cw {
                let off = (yy * cw + xx) * 3;
                let color = Color {
                    r: canvas[off],
                    g: canvas[off + 1],
                    b: canvas[off + 2],
                };
                w.set_pixel(cx + xx, cy + yy, color);
            }
        }
        w.draw_border(cx, cy, cw, ch, 1, MUTED);
        widgets::button_modern(
            w, cx, cy + ch + 10, 100, 30,
            "Clear",
            SURFACE_DARK_2,
            TEXT_LIGHT,
            false,
        );
    }

    fn draw_taskbar(&self, w: &mut Writer) {
        let h = w.height;
        let y = h.saturating_sub(TASKBAR_H);

        w.fill_rect(0, y, w.width, TASKBAR_H, Color { r: 22, g: 24, b: 32 });
        w.fill_rect(0, y, w.width, 1, Color { r: 60, g: 65, b: 80 });

        let start_hover = widgets::hit(
            self.hover.0, self.hover.1,
            6, y + 6, 70, TASKBAR_H.saturating_sub(12),
        );
        let start_bg = if self.start_pressed || start_hover {
            ACCENT
        } else {
            Color { r: 45, g: 48, b: 56 }
        };
        widgets::button_modern(
            w, 6, y + 6, 70, TASKBAR_H.saturating_sub(12),
            "Start", start_bg, Color::WHITE, false,
        );

        let mut bx = 86;
        for (idx, win) in self.windows.iter().enumerate() {
            let label = &win.title;
            let bw = Writer::text_width(label) + 24;
            let pressed = idx == self.active && !win.minimized;
            let bg = if pressed {
                ACCENT
            } else {
                Color { r: 45, g: 48, b: 56 }
            };
            widgets::button_modern(
                w, bx, y + 6, bw, TASKBAR_H.saturating_sub(12),
                label, bg, Color::WHITE, false,
            );
            bx += bw + 4;
        }

        let secs = uptime_secs();
        let text = format!("{:02}:{:02}", (secs / 60) % 60, secs % 60);
        let tw = Writer::text_width(&text);
        w.draw_text_at(
            w.width.saturating_sub(tw + 16),
            y + (TASKBAR_H - FONT_HEIGHT) / 2,
            &text,
            TEXT_LIGHT,
            Color { r: 22, g: 24, b: 32 },
        );
    }

    fn draw_start_menu(&self, w: &mut Writer) {
        let h = w.height;
        let menu_h = 240;
        let menu_w = 240;
        let menu_x = 6;
        let menu_y = h.saturating_sub(TASKBAR_H + menu_h + 6);

        w.fill_round_rect(
            menu_x + 4, menu_y + 4, menu_w, menu_h, 12,
            Color { r: 8, g: 10, b: 14 },
        );
        w.fill_round_rect(menu_x, menu_y, menu_w, menu_h, 12, SURFACE_DARK_2);

        w.draw_text_at(menu_x + 16, menu_y + 14, "Rust OS", TEXT_LIGHT, SURFACE_DARK_2);
        w.draw_text_at(
            menu_x + 16, menu_y + 14 + FONT_HEIGHT,
            "v0.5", MUTED, SURFACE_DARK_2,
        );

        let items = ["Programs", "Documents", "Settings", "Help", "Shutdown"];
        for (i, item) in items.iter().enumerate() {
            let iy = menu_y + 60 + i * 32;
            let hover = widgets::hit(
                self.hover.0, self.hover.1,
                menu_x + 8, iy, menu_w.saturating_sub(16), 28,
            );
            let bg = if hover { ACCENT } else { SURFACE_DARK_2 };
            w.fill_round_rect(menu_x + 8, iy, menu_w.saturating_sub(16), 28, 6, bg);
            w.draw_text_at(menu_x + 20, iy + 5, item, TEXT_LIGHT, bg);
        }
    }
}

// ---------- Calculator layout ----------

struct CalcLayout {
    display_x: usize,
    display_y: usize,
    display_w: usize,
    display_h: usize,
    grid_x: usize,
    grid_y: usize,
    step: usize,
}

fn calc_layout(win: &Window) -> CalcLayout {
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

// ---------- Cursor ----------

const CURSOR: &[(i32, i32, u8)] = &[
    (0,0,0),(1,1,0),(2,2,0),(3,3,0),(4,4,0),(5,5,0),(6,6,0),(7,7,0),(8,8,0),(9,9,0),(10,10,0),(11,11,0),(11,12,0),
    (1,1,1),(2,2,1),(3,3,1),(4,4,1),(5,5,1),(6,6,1),(7,7,1),(8,8,1),(9,9,1),(10,10,1),
    (2,10,0),(3,10,0),(4,10,0),(5,10,0),(6,10,0),(7,10,0),(8,10,0),
    (3,11,1),(4,11,1),(5,11,1),(6,11,1),(7,11,1),
    (3,12,1),(4,12,1),(5,12,1),(6,12,1),(7,12,1),
    (4,13,0),(5,13,0),(6,13,0),
    (4,14,1),(5,14,1),(6,14,1),
    (5,15,0),(6,15,0),
    (5,16,1),(6,16,1),
    (6,17,0),
    (6,18,1),
];

fn draw_cursor(w: &mut Writer, mx: i32, my: i32) {
    for &(dx, dy, c) in CURSOR {
        let x = mx + dx;
        let y = my + dy;
        if x < 0 || y < 0 { continue; }
        let color = if c == 0 { Color::DARK } else { Color::WHITE };
        w.set_pixel(x as usize, y as usize, color);
    }
}