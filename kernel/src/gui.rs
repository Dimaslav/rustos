use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::arch::asm;
use x86_64::instructions::interrupts;

use crate::framebuffer::{self, Color, Writer, FONT_HEIGHT};
use crate::interrupts::{ticks, uptime_secs};
use crate::keyboard::{self, Key};
use crate::mouse::{self, MOUSE};
use crate::sound;
use crate::widgets::{self, ACCENT, BUTTON_H, FIELD_H, MUTED, ROW_H, SURFACE_DARK, SURFACE_DARK_2, TEXT_LIGHT};

const TITLE_H: usize = 30;
const TASKBAR_H: usize = 42;
const CLOSE_BTN_W: usize = 26;
const MIN_BTN_W: usize = 26;

// Тёмный «стеклянный» фон рабочего стола
const WALLPAPER_TOP: Color = Color { r: 18, g: 22, b: 34 };
const WALLPAPER_BOTTOM: Color = Color { r: 40, g: 30, b: 60 };

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
    x: i32, y: i32, w: usize, h: usize,
    title: String,
    content: App,
    minimized: bool,
}

struct Drag { idx: usize, ox: i32, oy: i32 }

struct Wm {
    windows: Vec<Window>,
    active: usize,
    drag: Option<Drag>,
    start_pressed: bool,
    last_click: Option<(u64, AppKind)>,
    hover_x: i32, hover_y: i32,
}

static WM: spin::Mutex<Option<Wm>> = spin::Mutex::new(None);

const ICON_X: usize = 32;
const ICON_Y: usize = 32;
const ICON_STEP: usize = 110;

pub fn run() -> ! {
    let mut wm = Wm {
        windows: Vec::new(),
        active: 0,
        drag: None,
        start_pressed: false,
        last_click: None,
        hover_x: -1, hover_y: -1,
    };

    wm.windows.push(Window {
        x: 420, y: 200, w: 480, h: 220,
        title: "Добро пожаловать".to_string(),
        content: App::Notepad {
            text: "Rust OS v0.3\n\nДвойной клик по иконке открывает приложение.\nAlt+Tab — переключение окон.\nAlt+F4 — закрыть окно.".to_string(),
        },
        minimized: false,
    });
    wm.active = 0;
    *WM.lock() = Some(wm);

    loop {
        let (mx, my, dirty) = interrupts::without_interrupts(|| {
            let mut m = MOUSE.lock();
            let was = m.dirty;
            m.dirty = false;
            (m.x, m.y, was)
        });

        // Клавиатура
        while let Some(k) = keyboard::pop() {
            let (_, _, alt) = keyboard::modifiers();
            let mut g = WM.lock();
            let wm = g.as_mut().unwrap();
            wm.handle_key(k, alt);
        }

        // Клик (debounced)
        if let Some((cx, cy, _, _)) = mouse::take_click() {
            let mut g = WM.lock();
            let wm = g.as_mut().unwrap();
            wm.handle_click(cx, cy);
            sound::click();
        }

        // Движение/таскание
        {
            let mut g = WM.lock();
            let wm = g.as_mut().unwrap();
            wm.handle_move(mx, my);
        }

        let redraw = dirty || (ticks() % 18 == 0);
        if redraw {
            framebuffer::with_writer(|w| {
                let g = WM.lock();
                let wm = g.as_ref().unwrap();
                wm.draw(w, mx, my);
            });
            framebuffer::flush();
        }

        unsafe { asm!("hlt"); }
    }
}

impl Wm {
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
                if i != self.windows.len() - 1 {
                    let win = self.windows.remove(i);
                    self.windows.push(win);
                }
                self.active = self.windows.len() - 1;
                self.windows[self.active].minimized = false;
                sound::open();
                return;
            }
        }

        let (title, content) = match kind {
            AppKind::Notepad => ("Заметки", App::Notepad { text: String::new() }),
            AppKind::Todo => ("Задачи", App::Todo {
                items: vec!["Сверстать UI".to_string(), "Написать звук".to_string()],
                selected: None, input: String::new(),
            }),
            AppKind::Calculator => ("Калькулятор", App::Calculator {
                display: "0".to_string(), a: 0.0, op: ' ', fresh: true,
            }),
            AppKind::Paint => ("Paint", App::Paint {
                canvas: vec![255u8; 320 * 200 * 3],
                w: 320, h: 200, last: None,
            }),
        };

        let off = self.windows.len() as i32 * 28;
        self.windows.push(Window {
            x: 140 + off, y: 90 + off,
            w: 500, h: 340,
            title: title.to_string(),
            content,
            minimized: false,
        });
        self.active = self.windows.len() - 1;
        sound::open();
    }

    fn handle_key(&mut self, k: Key, alt: bool) {
        if alt && k == Key::Tab && !self.windows.is_empty() {
            self.active = (self.active + 1) % self.windows.len();
            self.windows[self.active].minimized = false;
            return;
        }
        if alt && k == Key::F(4) && !self.windows.is_empty() {
            self.close_active();
            return;
        }
        if k == Key::Escape { self.start_pressed = false; return; }

        if self.windows.is_empty() { return; }
        let idx = self.active;
        let win = &mut self.windows[idx];
        win.minimized = false;

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
                    if !s.is_empty() { items.push(s.to_string()); input.clear(); }
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
                *op = ch; *fresh = true;
            }
            '=' => {
                let b = display.parse::<f64>().unwrap_or(0.0);
                let r = match *op {
                    '+' => *a + b, '-' => *a - b, '*' => *a * b,
                    '/' => if b != 0.0 { *a / b } else { f64::NAN },
                    _ => b,
                };
                let rounded = r as i64;
                *display = if (r - rounded as f64).abs() < 0.0001 {
                    format!("{}", rounded)
                } else {
                    format!("{:.4}", r)
                };
                *op = ' '; *fresh = true;
            }
            'C' => {
                display.clear(); display.push('0');
                *a = 0.0; *op = ' '; *fresh = true;
            }
            _ => {}
        }
    }

    fn close_active(&mut self) {
        if self.windows.is_empty() { return; }
        self.windows.remove(self.active);
        if self.windows.is_empty() { self.active = 0; }
        else if self.active >= self.windows.len() { self.active = self.windows.len() - 1; }
    }

    fn icons() -> [(AppKind, &'static str, char, Color); 4] {
        [
            (AppKind::Notepad,    "Заметки",     'N', Color { r: 100, g: 150, b: 255 }),
            (AppKind::Todo,       "Задачи",      'T', Color { r: 80,  g: 200, b: 120 }),
            (AppKind::Calculator, "Калькулятор", 'C', Color { r: 240, g: 150, b: 60 }),
            (AppKind::Paint,      "Paint",       'P', Color { r: 220, g: 90,  b: 180 }),
        ]
    }

    fn icon_rect(i: usize) -> (usize, usize, usize, usize) {
        let col = i / 4;
        let row = i % 4;
        (ICON_X + col * ICON_STEP, ICON_Y + row * 90, 44, 66)
    }

    fn handle_move(&mut self, mx: i32, my: i32) {
        self.hover_x = mx;
        self.hover_y = my;
        if let Some(d) = &self.drag {
            let (idx, ox, oy) = (d.idx, d.ox, d.oy);
            let w = &mut self.windows[idx];
            w.x = mx - ox;
            w.y = my - oy;
        }
    }

    fn handle_click(&mut self, mx: i32, my: i32) {
        let (_, screen_h) = framebuffer::with_writer(|w| (w.width, w.height));
        let taskbar_y = screen_h - TASKBAR_H;

        // Панель задач
        if (my as usize) >= taskbar_y {
            if mx < 78 {
                self.start_pressed = !self.start_pressed;
                return;
            }
            // Кнопки окон в таскбаре
            let mut bx = 86;
            for (idx, w) in self.windows.iter().enumerate() {
                let label_w = Writer::text_width(&w.title) + 24;
                if (mx as usize) >= bx && (mx as usize) < bx + label_w {
                    if idx == self.active {
                        self.windows[idx].minimized = !self.windows[idx].minimized;
                    } else {
                        self.active = idx;
                        self.windows[idx].minimized = false;
                    }
                    return;
                }
                bx += label_w + 4;
            }
            return;
        }
        self.start_pressed = false;

        // Иконки
        let icons = Self::icons();
        for (i, (kind, _, _, _)) in icons.iter().enumerate() {
            let (ix, iy, iw, ih) = Self::icon_rect(i);
            if widgets::hit(mx, my, ix, iy, iw, ih) {
                let now = ticks();
                let dbl = matches!(self.last_click,
                    Some((t, k)) if k == *kind && now.saturating_sub(t) < 40);
                if dbl {
                    self.open_app(*kind);
                    self.last_click = None;
                } else {
                    self.last_click = Some((now, *kind));
                }
                return;
            }
        }

        // Окна сверху вниз
        let mut hit_idx = None;
        for idx in (0..self.windows.len()).rev() {
            if self.windows[idx].minimized { continue; }
            let w = &self.windows[idx];
            if widgets::hit(mx, my, w.x as usize, w.y as usize, w.w, w.h) {
                hit_idx = Some(idx);
                break;
            }
        }

        if let Some(idx) = hit_idx {
            if idx != self.windows.len() - 1 {
                let win = self.windows.remove(idx);
                self.windows.push(win);
                self.active = self.windows.len() - 1;
            } else {
                self.active = idx;
            }

            let (wx, wy, ww) = {
                let w = &self.windows[self.active];
                (w.x, w.y, w.w)
            };
            let rel_x = (mx - wx) as usize;
            let rel_y = (my - wy) as usize;

            if rel_y < TITLE_H {
                let cb_x = ww - 8 - CLOSE_BTN_W;
                let mb_x = cb_x - 6 - MIN_BTN_W;
                if rel_x >= cb_x {
                    self.close_active();
                } else if rel_x >= mb_x {
                    let i = self.active;
                    self.windows[i].minimized = true;
                } else {
                    self.drag = Some(Drag { idx: self.active, ox: mx - wx, oy: my - wy });
                }
            } else {
                self.handle_content_click(mx, my);
            }
        } else {
            self.drag = None;
        }
    }

    fn handle_content_click(&mut self, mx: i32, my: i32) {
        let win = &mut self.windows[self.active];
        let wx = win.x as usize;
        let wy = win.y as usize;

        match &mut win.content {
            App::Notepad { .. } => {}
            App::Todo { items, selected, input } => {
                let pad = 16;
                let field_x = wx + pad;
                let field_y = wy + TITLE_H + pad;
                let field_w = win.w - pad * 3 - 80;
                let add_btn_x = field_x + field_w + 8;
                let list_x = wx + pad;
                let list_y = field_y + FIELD_H + 12;
                let list_w = win.w - pad * 2;
                let list_h = win.h - TITLE_H - FIELD_H - 4 * pad - BUTTON_H;
                let remove_btn_y = list_y + list_h + 10;

                if widgets::hit(mx, my, add_btn_x, field_y, 72, FIELD_H) {
                    let s = input.trim();
                    if !s.is_empty() { items.push(s.to_string()); input.clear(); }
                    return;
                }
                if widgets::hit(mx, my, list_x, remove_btn_y, 120, BUTTON_H) {
                    if let Some(sel) = selected.take() {
                        if sel < items.len() { items.remove(sel); }
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
                let (bx, by, bw, bh, gap) = (wx + 16, wy + TITLE_H + 70, 64, 44, 6);
                let labels = [
                    ["7","8","9","/"],
                    ["4","5","6","*"],
                    ["1","2","3","-"],
                    ["0",".","=","+"],
                ];
                for (r, row) in labels.iter().enumerate() {
                    for (c, label) in row.iter().enumerate() {
                        let kx = bx + c * (bw + gap);
                        let ky = by + r * (bh + gap);
                        if widgets::hit(mx, my, kx, ky, bw, bh) {
                            self.calc_input(label.chars().next().unwrap());
                            return;
                        }
                    }
                }
                let cx = bx;
                let cy = by - 50;
                if widgets::hit(mx, my, cx, cy, bw, 40) {
                    self.calc_input('C');
                }
            }
            App::Paint { canvas, w, h, last } => {
                let pad = 10;
                let cx = wx + pad;
                let cy = wy + TITLE_H + pad;
                let cw = *w; let ch = *h;
                let clear_y = cy + ch + 10;
                if widgets::hit(mx, my, cx, clear_y, 100, 30) {
                    for p in canvas.iter_mut() { *p = 255; }
                    return;
                }
                if widgets::hit(mx, my, cx, cy, cw, ch) {
                    let px = (mx as usize - cx).min(cw - 1);
                    let py = (my as usize - cy).min(ch - 1);
                    *last = Some((px, py));
                    let idx = self.active;
                    // помечаем, что рисование начато
                    let _ = idx;
                }
            }
        }
    }

    fn draw(&self, w: &mut Writer, mx: i32, my: i32) {
        // Обои: вертикальный градиент
        w.gradient_v(0, 0, w.width, w.height, WALLPAPER_TOP, WALLPAPER_BOTTOM);

        // Иконки рабочего стола
        let icons = Self::icons();
        let selected_kind = self.last_click.map(|(_, k)| k);
        for (i, (kind, label, ch, color)) in icons.iter().enumerate() {
            let (ix, iy, _, _) = Self::icon_rect(i);
            let sel = selected_kind == Some(*kind);
            widgets::desktop_icon_modern(w, ix, iy, label, *ch, *color, sel);
        }

        // Окна
        for (idx, win) in self.windows.iter().enumerate() {
            if win.minimized { continue; }
            let active = idx == self.active;
            self.draw_window(w, win, active);
        }

        // Панель задач
        self.draw_taskbar(w);

        if self.start_pressed { self.draw_start_menu(w); }

        draw_cursor(w, mx, my);
    }

    fn draw_window(&self, w: &mut Writer, win: &Window, active: bool) {
        let (x, y, ww, wh) = (win.x as usize, win.y as usize, win.w, win.h);

        // Тень под окном
        for i in 1..=8 {
            let alpha = (255 - (i * 28)) as u32;
            let c = Color {
                r: (0 * alpha / 255) as u8,
                g: (0 * alpha / 255) as u8,
                b: (0 * alpha / 255) as u8,
            };
            w.fill_round_rect(x + i, y + i + 2, ww, wh, 10, c);
        }

        // Тело окна
        w.fill_round_rect(x, y, ww, wh, 10, SURFACE_DARK);

        // Заголовок
        let title_bg = if active { ACCENT } else { SURFACE_DARK_2 };
        // Верхняя часть со скруглением
        w.fill_round_rect(x, y, ww, TITLE_H + 10, 10, title_bg);
        // «Отрезаем» нижнюю половину радиуса
        w.fill_rect(x, y + TITLE_H, ww, 10, SURFACE_DARK);

        w.draw_text_at(x + 14, y + (TITLE_H - FONT_HEIGHT) / 2, &win.title, TEXT_LIGHT, title_bg);

        // Кнопки
        let cb_x = x + ww - 12 - CLOSE_BTN_W;
        let cb_y = y + (TITLE_H - 20) / 2;
        self.draw_close_btn(w, cb_x, cb_y);
        let mb_x = cb_x - 6 - MIN_BTN_W;
        self.draw_min_btn(w, mb_x, cb_y);

        match &win.content {
            App::Notepad { text } => self.draw_notepad(w, win, text),
            App::Todo { items, selected, input } => self.draw_todo(w, win, items, *selected, input),
            App::Calculator { display, .. } => self.draw_calc(w, win, display),
            App::Paint { canvas, w: cw, h: chh, .. } => self.draw_paint(w, win, canvas, *cw, *chh),
        }
    }

    fn draw_close_btn(&self, w: &mut Writer, x: usize, y: usize) {
        let hover = widgets::hit(self.hover_x, self.hover_y, x, y, CLOSE_BTN_W, 20);
        let bg = if hover { Color { r: 232, g: 68, b: 68 } } else { SURFACE_DARK_2 };
        w.fill_round_rect(x, y, CLOSE_BTN_W, 20, 6, bg);
        let cx = x + CLOSE_BTN_W / 2;
        let cy = y + 10;
        w.fill_rect(cx - 3, cy - 1, 7, 2, TEXT_LIGHT);
        w.fill_rect(cx - 1, cy - 3, 2, 7, TEXT_LIGHT);
    }

    fn draw_min_btn(&self, w: &mut Writer, x: usize, y: usize) {
        let hover = widgets::hit(self.hover_x, self.hover_y, x, y, MIN_BTN_W, 20);
        let bg = if hover { Color { r: 80, g: 90, b: 110 } } else { SURFACE_DARK_2 };
        w.fill_round_rect(x, y, MIN_BTN_W, 20, 6, bg);
        let cx = x + MIN_BTN_W / 2;
        let cy = y + 12;
        w.fill_rect(cx - 4, cy, 8, 2, TEXT_LIGHT);
    }

    fn draw_notepad(&self, w: &mut Writer, win: &Window, text: &str) {
        let x = win.x as usize; let y = win.y as usize;
        let pad = 14;
        let field_x = x + pad;
        let field_y = y + TITLE_H + 6;
        let field_w = win.w - pad * 2;
        let field_h = win.h - TITLE_H - pad - 8;
        w.fill_round_rect(field_x, field_y, field_w, field_h, 6, SURFACE_DARK_2);

        let px = field_x + 8;
        let py = field_y + 8;
        let max_chars = (field_w.saturating_sub(16)) / 9;
        let mut line = 0usize; let mut col = 0usize;
        for c in text.chars() {
            if c == '\n' { line += 1; col = 0; if py + (line + 1) * FONT_HEIGHT > field_y + field_h { break; } continue; }
            if col >= max_chars { line += 1; col = 0; if py + (line + 1) * FONT_HEIGHT > field_y + field_h { break; } }
            w.draw_text_at(px + col * 9, py + line * FONT_HEIGHT, &c.to_string(), TEXT_LIGHT, SURFACE_DARK_2);
            col += 1;
        }
    }

    fn draw_todo(&self, w: &mut Writer, win: &Window, items: &[String], selected: Option<usize>, input: &str) {
        let x = win.x as usize; let y = win.y as usize;
        let pad = 16;
        let field_x = x + pad;
        let field_y = y + TITLE_H + pad;
        let field_w = win.w - pad * 3 - 80;
        let add_btn_x = field_x + field_w + 8;
        widgets::text_field_modern(w, field_x, field_y, field_w, FIELD_H, input, true);
        widgets::button_modern(w, add_btn_x, field_y, 72, FIELD_H, "Добавить", ACCENT, Color::WHITE, false);
        let list_x = x + pad;
        let list_y = field_y + FIELD_H + 12;
        let list_w = win.w - pad * 2;
        let list_h = win.h - TITLE_H - FIELD_H - 4 * pad - BUTTON_H;
        widgets::list_box_modern(w, list_x, list_y, list_w, list_h, items, selected);
        let remove_btn_y = list_y + list_h + 10;
        widgets::button_modern(w, list_x, remove_btn_y, 120, BUTTON_H, "Удалить", Color { r: 180, g: 60, b: 60 }, Color::WHITE, false);
    }

    fn draw_calc(&self, w: &mut Writer, win: &Window, display: &str) {
        let x = win.x as usize; let y = win.y as usize;
        let pad = 16;
        let fx = x + pad;
        let fy = y + TITLE_H + pad;
        let fw = win.w - pad * 2;
        let fh = 42;
        w.fill_round_rect(fx, fy, fw, fh, 6, SURFACE_DARK_2);
        let tw = Writer::text_width(display);
        w.draw_text_at(fx + fw - tw - 12, fy + (fh - FONT_HEIGHT) / 2, display, TEXT_LIGHT, SURFACE_DARK_2);

        let bx = x + pad;
        let by = fy + fh + 12;
        widgets::button_modern(w, bx, by, 64, 40, "C", Color { r: 180, g: 60, b: 60 }, Color::WHITE, false);

        let gx = bx + 70;
        let gy = by;
        let labels = [
            ["7","8","9","/"],
            ["4","5","6","*"],
            ["1","2","3","-"],
            ["0",".","=","+"],
        ];
        let bw = 64; let bh = 44; let gap = 6;
        for (r, row) in labels.iter().enumerate() {
            for (c, label) in row.iter().enumerate() {
                let is_op = *label == "/" || *label == "*" || *label == "-" || *label == "+" || *label == "=";
                let bg = if is_op { ACCENT } else { SURFACE_DARK_2 };
                widgets::button_modern(w, gx + c * (bw + gap), gy + r * (bh + gap), bw, bh, label, bg, Color::WHITE, false);
            }
        }
    }

    fn draw_paint(&self, w: &mut Writer, win: &Window, canvas: &[u8], cw: usize, ch: usize) {
        let x = win.x as usize; let y = win.y as usize;
        let pad = 10;
        let cx = x + pad;
        let cy = y + TITLE_H + pad;
        for yy in 0..ch {
            for xx in 0..cw {
                let off = (yy * cw + xx) * 3;
                let color = Color { r: canvas[off], g: canvas[off + 1], b: canvas[off + 2] };
                w.set_pixel(cx + xx, cy + yy, color);
            }
        }
        w.draw_border(cx, cy, cw, ch, 1, MUTED);
        widgets::button_modern(w, cx, cy + ch + 10, 100, 30, "Очистить", SURFACE_DARK_2, TEXT_LIGHT, false);
    }

    fn draw_taskbar(&self, w: &mut Writer) {
        let h = w.height;
        let y = h - TASKBAR_H;

        // «Стеклянная» панель
        w.fill_rect(0, y, w.width, TASKBAR_H, Color { r: 22, g: 24, b: 32 });
        w.fill_rect(0, y, w.width, 1, Color { r: 60, g: 65, b: 80 });

        // Кнопка Пуск
        let start_hover = widgets::hit(self.hover_x, self.hover_y, 6, y + 6, 70, TASKBAR_H - 12);
        let start_bg = if self.start_pressed || start_hover { ACCENT } else { Color { r: 45, g: 48, b: 56 } };
        widgets::button_modern(w, 6, y + 6, 70, TASKBAR_H - 12, "Пуск", start_bg, Color::WHITE, false);

        // Кнопки окон
        let mut bx = 86;
        for (idx, win) in self.windows.iter().enumerate() {
            let label = &win.title;
            let bw = Writer::text_width(label) + 24;
            let pressed = idx == self.active && !win.minimized;
            let bg = if pressed { ACCENT } else { Color { r: 45, g: 48, b: 56 } };
            widgets::button_modern(w, bx, y + 6, bw, TASKBAR_H - 12, label, bg, Color::WHITE, false);
            bx += bw + 4;
        }

        // Часы
        let secs = uptime_secs();
        let text = format!("{:02}:{:02}", (secs / 60) % 60, secs % 60);
        let tw = Writer::text_width(&text);
        w.draw_text_at(w.width - tw - 16, y + (TASKBAR_H - FONT_HEIGHT) / 2, &text, TEXT_LIGHT, Color { r: 22, g: 24, b: 32 });
    }

    fn draw_start_menu(&self, w: &mut Writer) {
        let h = w.height;
        let menu_h = 240;
        let menu_w = 240;
        let menu_x = 6;
        let menu_y = h - TASKBAR_H - menu_h - 6;

        // Тень
        w.fill_round_rect(menu_x + 4, menu_y + 4, menu_w, menu_h, 12, Color { r: 0, g: 0, b: 0 });
        // Фон
        w.fill_round_rect(menu_x, menu_y, menu_w, menu_h, 12, SURFACE_DARK_2);

        // Заголовок
        w.draw_text_at(menu_x + 16, menu_y + 14, "Rust OS", TEXT_LIGHT, SURFACE_DARK_2);
        w.draw_text_at(menu_x + 16, menu_y + 14 + FONT_HEIGHT, "v0.3", MUTED, SURFACE_DARK_2);

        let items = ["Программы", "Документы", "Настройки", "Справка", "Выключить"];
        for (i, item) in items.iter().enumerate() {
            let iy = menu_y + 60 + i * 32;
            let hover = widgets::hit(self.hover_x, self.hover_y, menu_x + 8, iy, menu_w - 16, 28);
            let bg = if hover { ACCENT } else { SURFACE_DARK_2 };
            w.fill_round_rect(menu_x + 8, iy, menu_w - 16, 28, 6, bg);
            w.draw_text_at(menu_x + 20, iy + 5, item, TEXT_LIGHT, bg);
        }
    }
}

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
    for &(dx, dy, c) in &CURSOR[..] {
        let x = mx + dx; let y = my + dy;
        if x < 0 || y < 0 { continue; }
        let color = if c == 0 { Color::DARK } else { Color::WHITE };
        w.set_pixel(x as usize, y as usize, color);
    }
}