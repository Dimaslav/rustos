use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use crate::fat32::{Fat32, FatKind};
use crate::framebuffer::{self, Color, Writer, FONT_HEIGHT};
use crate::fs::{self, FileSystem, NodeKind};
use crate::interrupts::{ticks, uptime_secs, TIMER_HZ};
use crate::keyboard::{self, Key};
use crate::mouse::{self, MouseButton, MouseEvent};
use crate::sound;
use crate::widgets::{
    self, ACCENT, BUTTON_H, FIELD_H, MUTED, ROW_H, SURFACE_DARK, SURFACE_DARK_2, TEXT_LIGHT,
};

pub static DISK: spin::Mutex<Option<Fat32>> = spin::Mutex::new(None);

const TITLE_H: usize = 30;
const TASKBAR_H: usize = 42;
const CLOSE_BTN_W: usize = 26;
const MIN_BTN_W: usize = 26;

const WALLPAPER_TOP: Color = Color { r: 18, g: 22, b: 34 };
const WALLPAPER_BOTTOM: Color = Color { r: 40, g: 30, b: 60 };

const ICON_X: usize = 32;
const ICON_Y: usize = 32;
const ICON_STEP: usize = 110;

const EXP_TOOLBAR_H: usize = 38;
const EXP_ROW_H: usize = 28;
const EXP_SIDEBAR_W: usize = 160;

#[derive(Clone)]
enum ExplorerMode {
    Browse,
    NewFolder { name: String },
    Rename { name: String },
}

#[derive(Clone)]
enum App {
    Notepad {
        text: String,
        file: Option<String>,
        modified: bool,
    },
    Explorer {
        path: String,
        selected: Option<usize>,
        history: Vec<String>,
        mode: ExplorerMode,
        last_click: Option<(u64, usize)>,
    },
    Todo {
        items: Vec<String>,
        selected: Option<usize>,
        input: String,
    },
    Calculator {
        display: String,
        a: f64,
        op: char,
        fresh: bool,
    },
    Paint {
        canvas: Vec<u8>,
        w: usize,
        h: usize,
        last: Option<(usize, usize)>,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AppKind {
    Explorer,
    Notepad,
    Todo,
    Calculator,
    Paint,
}

struct Window {
    x: i32,
    y: i32,
    w: usize,
    h: usize,
    title: String,
    content: App,
    minimized: bool,
}

struct Drag {
    idx: usize,
    ox: i32,
    oy: i32,
}

struct Wm {
    windows: Vec<Window>,
    active: usize,
    fs: FileSystem,
    drag: Option<Drag>,
    painting: Option<usize>,
    start_pressed: bool,
    last_click: Option<(u64, AppKind)>,
    hover: (i32, i32),
    dirty: bool,
    cursor_dirty: bool,
}

pub fn run() -> ! {
    let (sw, sh) = framebuffer::with_writer(|w| (w.width, w.height));
    mouse::set_screen(sw, sh);

    let mut wm = Wm::new();
    let mut last_pos = mouse::position();
    wm.hover = last_pos;
    wm.dirty = true;
    wm.cursor_dirty = false;
    let mut last_stat_tick = ticks();

    loop {
        sound::tick();

        let pos = mouse::position();
        if pos != last_pos {
            last_pos = pos;
            wm.on_move(pos.0, pos.1);
            wm.cursor_dirty = true;
        }

        while let Some(ev) = mouse::pop_event() {
            match ev {
                MouseEvent::ButtonDown(MouseButton::Left) => {
                    wm.on_button_down(pos.0, pos.1);
                    wm.dirty = true;
                }
                MouseEvent::ButtonUp(MouseButton::Left) => {
                    wm.on_button_up(pos.0, pos.1);
                    wm.dirty = true;
                }
                _ => {}
            }
        }

        while let Some(k) = keyboard::pop() {
            let (_, ctrl, alt) = keyboard::modifiers();
            wm.on_key(k, ctrl, alt);
            wm.dirty = true;
        }

        let now = ticks();
        if now.saturating_sub(last_stat_tick) >= TIMER_HZ {
            last_stat_tick = now;
            use core::sync::atomic::Ordering;
            crate::serial_println!(
                "[stat] tick={} irq_starts={} bytes_irq={} packets={}",
                now,
                mouse::IRQ_STARTS.load(Ordering::Relaxed),
                mouse::BYTES_IRQ.load(Ordering::Relaxed),
                mouse::PACKETS_DONE.load(Ordering::Relaxed),
            );
        }

        if wm.dirty {
            framebuffer::with_writer(|w| {
                wm.draw(w);
                w.end_scene();
                let _ = w.move_cursor(pos.0, pos.1, CURSOR);
                w.flush_all();
            });
            wm.dirty = false;
            wm.cursor_dirty = false;
        } else if wm.cursor_dirty {
            framebuffer::with_writer(|w| {
                let (r1, r2) = w.move_cursor(pos.0, pos.1, CURSOR);
                w.flush_rect(r1.0, r1.1, r1.2, r1.3);
                w.flush_rect(r2.0, r2.1, r2.2, r2.3);
            });
            wm.cursor_dirty = false;
        }

        x86_64::instructions::interrupts::enable_and_hlt();
    }
}

impl Wm {
    fn new() -> Self {
        let mut wm = Wm {
            windows: Vec::new(),
            active: 0,
            fs: FileSystem::new(),
            drag: None,
            painting: None,
            start_pressed: false,
            last_click: None,
            hover: mouse::position(),
            dirty: true,
            cursor_dirty: false,
        };
        wm.open_app(AppKind::Explorer);
        wm
    }

    fn icons() -> [(AppKind, &'static str, char, Color); 5] {
        [
            (AppKind::Explorer,   "Files",      'F', Color { r: 255, g: 195, b: 70 }),
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
                (App::Explorer { .. }, AppKind::Explorer)
                    | (App::Notepad { .. }, AppKind::Notepad)
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
        self.spawn_app(kind, None);
    }

    fn open_notepad_with_file(&mut self, path: String) {
        let content = self
            .fs
            .read(&path)
            .map(|v| String::from_utf8_lossy(&v).to_string())
            .unwrap_or_default();
        self.spawn_app(AppKind::Notepad, Some((path, content)));
    }

    fn spawn_app(&mut self, kind: AppKind, notepad_data: Option<(String, String)>) {
        let (title, content): (String, App) = match kind {
            AppKind::Explorer => (
                "File Explorer".to_string(),
                App::Explorer {
                    path: "/".to_string(),
                    selected: None,
                    history: Vec::new(),
                    mode: ExplorerMode::Browse,
                    last_click: None,
                },
            ),
            AppKind::Notepad => {
                if let Some((file, text)) = notepad_data {
                    let title = file.rsplit('/').next().unwrap_or("Notepad").to_string();
                    (
                        title,
                        App::Notepad {
                            text,
                            file: Some(file),
                            modified: false,
                        },
                    )
                } else {
                    (
                        "Notepad".to_string(),
                        App::Notepad {
                            text: String::new(),
                            file: None,
                            modified: false,
                        },
                    )
                }
            }
            AppKind::Todo => (
                "Tasks".to_string(),
                App::Todo {
                    items: vec!["Write UI".to_string(), "Add sound".to_string()],
                    selected: None,
                    input: String::new(),
                },
            ),
            AppKind::Calculator => (
                "Calculator".to_string(),
                App::Calculator {
                    display: "0".to_string(),
                    a: 0.0,
                    op: ' ',
                    fresh: true,
                },
            ),
            AppKind::Paint => (
                "Paint".to_string(),
                App::Paint {
                    canvas: vec![255u8; 320 * 200 * 3],
                    w: 320,
                    h: 200,
                    last: None,
                },
            ),
        };

        let (ww, wh) = if kind == AppKind::Explorer { (720, 460) } else { (500, 340) };

        let off = self.windows.len() as i32 * 28;
        self.windows.push(Window {
            x: 100 + off,
            y: 70 + off,
            w: ww,
            h: wh,
            title,
            content,
            minimized: false,
        });
        self.active = self.windows.len() - 1;
        self.dirty = true;
        sound::open();
    }

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
                    self.drag = Some(Drag { idx: self.active, ox: x - wx, oy: y - wy });
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

    fn on_key(&mut self, k: Key, ctrl: bool, alt: bool) {
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
            let idx = self.active;
            if let App::Explorer { mode, .. } = &mut self.windows[idx].content {
                if !matches!(mode, ExplorerMode::Browse) {
                    *mode = ExplorerMode::Browse;
                    self.dirty = true;
                    return;
                }
            }
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
            App::Notepad { text, file: _, modified } => match k {
                Key::Enter => { text.push('\n'); *modified = true; }
                Key::Backspace => { text.pop(); *modified = true; }
                Key::Tab => { text.push_str("    "); *modified = true; }
                Key::Char(c) => { text.push(c as char); *modified = true; }
                _ => {}
            },
            App::Explorer { path, selected, history, mode, .. } => {
                match mode {
                    ExplorerMode::NewFolder { name } => match k {
                        Key::Enter => {
                            let n = name.clone();
                            let p = path.clone();
                            if !n.is_empty() && self.fs.mkdir(&p, &n) {
                                *mode = ExplorerMode::Browse;
                            }
                        }
                        Key::Backspace => { name.pop(); }
                        Key::Char(c) => name.push(c as char),
                        _ => {}
                    },
                    ExplorerMode::Rename { name } => match k {
                        Key::Enter => {
                            let n = name.clone();
                            let p = path.clone();
                            let sel = *selected;
                            if let Some(i) = sel {
                                let entries = self.fs.list(&p);
                                if i < entries.len() {
                                    let full = fs::join(&p, &entries[i].name);
                                    if self.fs.rename(&full, &n) {
                                        *mode = ExplorerMode::Browse;
                                    }
                                }
                            }
                        }
                        Key::Backspace => { name.pop(); }
                        Key::Char(c) => name.push(c as char),
                        _ => {}
                    },
                    ExplorerMode::Browse => {
                        match k {
                            Key::Backspace => {
                                let cur = path.clone();
                                if cur != "/" && !cur.starts_with("C:") {
                                    let parent = fs::parent_path(&cur);
                                    history.push(cur);
                                    *path = parent;
                                    *selected = None;
                                }
                            }
                            Key::Enter => {
                                if let Some(i) = *selected {
                                    let entries = self.fs.list(path);
                                    if let Some(e) = entries.get(i) {
                                        let full = fs::join(path, &e.name);
                                        if e.kind == NodeKind::Directory {
                                            history.push(path.clone());
                                            *path = full;
                                            *selected = None;
                                        }
                                    }
                                }
                            }
                            Key::Delete => {
                                if !path.starts_with("C:") {
                                    if let Some(i) = *selected {
                                        let entries = self.fs.list(path);
                                        if let Some(e) = entries.get(i) {
                                            let full = fs::join(path, &e.name);
                                            self.fs.remove(&full);
                                            *selected = None;
                                        }
                                    }
                                }
                            }
                            Key::F(2) => {
                                if !path.starts_with("C:") {
                                    if let Some(i) = *selected {
                                        let entries = self.fs.list(path);
                                        if let Some(e) = entries.get(i) {
                                            *mode = ExplorerMode::Rename { name: e.name.clone() };
                                        }
                                    }
                                }
                            }
                            Key::Char(c) if ctrl && (c == b'n' || c == b'N') => {
                                if !path.starts_with("C:") {
                                    *mode = ExplorerMode::NewFolder { name: String::new() };
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
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
        let is_explorer = matches!(&self.windows[active].content, App::Explorer { .. });
        if is_explorer {
            self.handle_explorer_click(mx, my);
            return;
        }

        let win = &mut self.windows[active];
        let wx = win.x.max(0) as usize;
        let wy = win.y.max(0) as usize;

        match &mut win.content {
            App::Notepad { .. } => {}
            App::Explorer { .. } => unreachable!(),
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

    fn handle_explorer_click(&mut self, mx: i32, my: i32) {
        let idx = self.active;
        let win = &mut self.windows[idx];
        let wx = win.x.max(0) as usize;
        let wy = win.y.max(0) as usize;
        let ww = win.w;

        let toolbar_y = wy + TITLE_H + 8;
        let body_y = toolbar_y + EXP_TOOLBAR_H + 8;

        let back_x = wx + 10;
        let up_x = back_x + 48;
        let nf_x = up_x + 48;
        let del_x = nf_x + 90;
        let ren_x = del_x + 80;

        // Диалог открыт? Клик отменяет.
        let is_dialog_open = matches!(
            &self.windows[idx].content,
            App::Explorer { mode, .. } if !matches!(mode, ExplorerMode::Browse)
        );
        if is_dialog_open {
            if let App::Explorer { mode, .. } = &mut self.windows[idx].content {
                *mode = ExplorerMode::Browse;
            }
            self.dirty = true;
            return;
        }

        // ---- Toolbar ----
        if (my as usize) >= toolbar_y && (my as usize) < toolbar_y + EXP_TOOLBAR_H {
            if widgets::hit(mx, my, back_x, toolbar_y, 42, EXP_TOOLBAR_H) {
                if let App::Explorer { path, history, selected, .. } = &mut self.windows[idx].content {
                    if let Some(prev) = history.pop() {
                        *path = prev;
                        *selected = None;
                    }
                }
                self.dirty = true;
                return;
            }
            if widgets::hit(mx, my, up_x, toolbar_y, 42, EXP_TOOLBAR_H) {
                if let App::Explorer { path, history, selected, .. } = &mut self.windows[idx].content {
                    let cur = path.clone();
                    if cur != "/" && !cur.starts_with("C:") {
                        let parent = fs::parent_path(&cur);
                        history.push(cur);
                        *path = parent;
                        *selected = None;
                    }
                }
                self.dirty = true;
                return;
            }
            if widgets::hit(mx, my, nf_x, toolbar_y, 86, EXP_TOOLBAR_H) {
                let on_disk = matches!(&self.windows[idx].content, App::Explorer { path, .. } if path.starts_with("C:"));
                if !on_disk {
                    if let App::Explorer { mode, .. } = &mut self.windows[idx].content {
                        *mode = ExplorerMode::NewFolder { name: String::new() };
                    }
                }
                self.dirty = true;
                return;
            }
            if widgets::hit(mx, my, del_x, toolbar_y, 76, EXP_TOOLBAR_H) {
                let on_disk = matches!(&self.windows[idx].content, App::Explorer { path, .. } if path.starts_with("C:"));
                if !on_disk {
                    let p = match &self.windows[idx].content {
                        App::Explorer { path, .. } => path.clone(),
                        _ => return,
                    };
                    let sel = match &self.windows[idx].content {
                        App::Explorer { selected, .. } => *selected,
                        _ => None,
                    };
                    if let Some(i) = sel {
                        let entries = self.fs.list(&p);
                        if let Some(e) = entries.get(i) {
                            let full = fs::join(&p, &e.name);
                            self.fs.remove(&full);
                            if let App::Explorer { selected, .. } = &mut self.windows[idx].content {
                                *selected = None;
                            }
                        }
                    }
                }
                self.dirty = true;
                return;
            }
            if widgets::hit(mx, my, ren_x, toolbar_y, 76, EXP_TOOLBAR_H) {
                let on_disk = matches!(&self.windows[idx].content, App::Explorer { path, .. } if path.starts_with("C:"));
                if !on_disk {
                    let p = match &self.windows[idx].content {
                        App::Explorer { path, .. } => path.clone(),
                        _ => return,
                    };
                    let sel = match &self.windows[idx].content {
                        App::Explorer { selected, .. } => *selected,
                        _ => None,
                    };
                    if let Some(i) = sel {
                        let entries = self.fs.list(&p);
                        if let Some(e) = entries.get(i) {
                            if let App::Explorer { mode, .. } = &mut self.windows[idx].content {
                                *mode = ExplorerMode::Rename { name: e.name.clone() };
                            }
                        }
                    }
                }
                self.dirty = true;
                return;
            }
            return;
        }

        // ---- Sidebar ----
        let sidebar_x = wx + 10;
        let sidebar_w = EXP_SIDEBAR_W;
        if (mx as usize) >= sidebar_x
            && (mx as usize) < sidebar_x + sidebar_w
            && (my as usize) >= body_y
        {
            let items: [(&str, &str); 6] = [
                ("Home", "/"),
                ("Desktop", "/Desktop"),
                ("Documents", "/Documents"),
                ("Downloads", "/Downloads"),
                ("System", "/System"),
                ("Disk (C:)", "C:/"),
            ];
            for (i, (_, p)) in items.iter().enumerate() {
                let ry = body_y + 14 + i * 30;
                if (my as usize) >= ry && (my as usize) < ry + 26 {
                    if let App::Explorer { path, history, selected, .. } = &mut self.windows[idx].content {
                        let cur = path.clone();
                        if cur != *p {
                            history.push(cur);
                            *path = p.to_string();
                            *selected = None;
                        }
                    }
                    self.dirty = true;
                    return;
                }
            }
            return;
        }

        // ---- File view ----
        let view_x = wx + sidebar_w + 20;
        let view_y = body_y;
        let view_w = ww.saturating_sub(sidebar_w + 30);

        if (mx as usize) >= view_x
            && (mx as usize) < view_x + view_w
            && (my as usize) >= view_y + 34
        {
            let rel_y = (my as usize) - (view_y + 34);
            let row = rel_y / EXP_ROW_H;

            let p = match &self.windows[idx].content {
                App::Explorer { path, .. } => path.clone(),
                _ => return,
            };

            // Пользуемся общей функцией list_dir_entries, чтобы не дублировать.
            let entries = self.list_dir_entries(&p);

            if row < entries.len() {
                let now = ticks();
                let dbl = if let App::Explorer { last_click, .. } = &self.windows[idx].content {
                    matches!(last_click, Some((t, r)) if *r == row && now.saturating_sub(*t) < 30)
                } else {
                    false
                };

                if let App::Explorer { selected, last_click, .. } = &mut self.windows[idx].content {
                    *selected = Some(row);
                    *last_click = Some((now, row));
                }

                if dbl {
                    let (name, is_dir, _) = entries[row].clone();
                    if is_dir {
                        if let App::Explorer { path, history, selected, last_click, .. } = &mut self.windows[idx].content {
                            history.push(path.clone());
                            if p == "C:/" {
                                // В подкаталог диска пока не умеем — просто в корень.
                                *path = "C:/".to_string();
                            } else {
                                *path = fs::join(&p, &name);
                            }
                            *selected = None;
                            *last_click = None;
                        }
                    } else if name.ends_with(".txt") || name.ends_with(".TXT") {
                        // Если мы на диске — читаем через FAT, иначе через RAMFS.
                        if p.starts_with("C:") {
                            if let Some(text) = self.read_disk_file(&name) {
                                self.spawn_app(AppKind::Notepad, Some((alloc::format!("C:/{}", name), text)));
                            }
                        } else {
                            let full = fs::join(&p, &name);
                            self.open_notepad_with_file(full);
                        }
                    }
                }
                self.dirty = true;
            } else {
                if let App::Explorer { selected, .. } = &mut self.windows[idx].content {
                    *selected = None;
                }
                self.dirty = true;
            }
        }
    }

    /// Список содержимого директории: (имя, is_dir, size).
    fn list_dir_entries(&mut self, path: &str) -> Vec<(String, bool, u32)> {
        if path.starts_with("C:") {
            let mut guard = DISK.lock();
            if let Some(fat) = guard.as_mut() {
                fat.list_root()
                    .into_iter()
                    .map(|e| (e.name, e.kind == FatKind::Directory, e.size))
                    .collect()
            } else {
                Vec::new()
            }
        } else {
            self.fs
                .list(path)
                .into_iter()
                .map(|e| (e.name, e.kind == NodeKind::Directory, e.size as u32))
                .collect()
        }
    }

    fn read_disk_file(&mut self, name: &str) -> Option<String> {
        let mut guard = DISK.lock();
        let fat = guard.as_mut()?;
        let entries = fat.list_root();
        let entry = entries.into_iter().find(|e| e.name.eq_ignore_ascii_case(name))?;
        let data = fat.read_file(&entry);
        Some(String::from_utf8_lossy(&data).to_string())
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
            App::Notepad { text, file, modified } => {
                self.draw_notepad(w, win, text, file.as_deref(), *modified)
            }
            App::Explorer { path, selected, mode, .. } => {
                self.draw_explorer(w, win, path, *selected, mode)
            }
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
        let bg = if hover { Color { r: 232, g: 68, b: 68 } } else { SURFACE_DARK_2 };
        w.fill_round_rect(x, y, CLOSE_BTN_W, 20, 6, bg);
        let cx = x + CLOSE_BTN_W / 2;
        let cy = y + 10;
        w.fill_rect(cx - 3, cy - 1, 7, 2, TEXT_LIGHT);
        w.fill_rect(cx - 1, cy - 3, 2, 7, TEXT_LIGHT);
    }

    fn draw_min_btn(&self, w: &mut Writer, x: usize, y: usize) {
        let hover = widgets::hit(self.hover.0, self.hover.1, x, y, MIN_BTN_W, 20);
        let bg = if hover { Color { r: 80, g: 90, b: 110 } } else { SURFACE_DARK_2 };
        w.fill_round_rect(x, y, MIN_BTN_W, 20, 6, bg);
        let cx = x + MIN_BTN_W / 2;
        let cy = y + 12;
        w.fill_rect(cx - 4, cy, 8, 2, TEXT_LIGHT);
    }

    fn draw_notepad(
        &self,
        w: &mut Writer,
        win: &Window,
        text: &str,
        file: Option<&str>,
        modified: bool,
    ) {
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

        let status = match (file, modified) {
            (Some(f), true) => format!("{} *", f),
            (Some(f), false) => f.to_string(),
            (None, _) => "(unsaved)".to_string(),
        };
        w.draw_text_at(
            field_x + 4,
            y + win.h.saturating_sub(14),
            &status,
            MUTED,
            SURFACE_DARK,
        );
    }

    fn draw_explorer(
        &self,
        w: &mut Writer,
        win: &Window,
        path: &str,
        selected: Option<usize>,
        mode: &ExplorerMode,
    ) {
        let x = win.x.max(0) as usize;
        let y = win.y.max(0) as usize;
        let ww = win.w;
        let wh = win.h;

        // ---- Toolbar ----
        let toolbar_y = y + TITLE_H + 8;
        let back_x = x + 10;
        let up_x = back_x + 48;
        let nf_x = up_x + 48;
        let del_x = nf_x + 90;
        let ren_x = del_x + 80;

        widgets::button_modern(w, back_x, toolbar_y, 42, EXP_TOOLBAR_H, "<", SURFACE_DARK_2, TEXT_LIGHT, false);
        widgets::button_modern(w, up_x, toolbar_y, 42, EXP_TOOLBAR_H, "^", SURFACE_DARK_2, TEXT_LIGHT, false);
        widgets::button_modern(w, nf_x, toolbar_y, 86, EXP_TOOLBAR_H, "+ Folder", SURFACE_DARK_2, TEXT_LIGHT, false);
        widgets::button_modern(w, del_x, toolbar_y, 76, EXP_TOOLBAR_H, "Delete", SURFACE_DARK_2, TEXT_LIGHT, false);
        widgets::button_modern(w, ren_x, toolbar_y, 76, EXP_TOOLBAR_H, "Rename", SURFACE_DARK_2, TEXT_LIGHT, false);

        // ---- Address bar ----
        let addr_x = ren_x + 86;
        let addr_w = (x + ww).saturating_sub(addr_x + 12);
        w.fill_round_rect(addr_x, toolbar_y + 4, addr_w, EXP_TOOLBAR_H - 8, 5, SURFACE_DARK_2);
        w.draw_text_at(
            addr_x + 10,
            toolbar_y + 4 + (EXP_TOOLBAR_H - 8 - FONT_HEIGHT) / 2,
            path,
            TEXT_LIGHT,
            SURFACE_DARK_2,
        );

        // ---- Body ----
        let body_y = toolbar_y + EXP_TOOLBAR_H + 8;
        let body_h = (y + wh).saturating_sub(body_y + 12);

        let sidebar_x = x + 10;
        w.fill_round_rect(sidebar_x, body_y, EXP_SIDEBAR_W, body_h, 6, SURFACE_DARK_2);
        let items: [&str; 6] = ["Home", "Desktop", "Documents", "Downloads", "System", "Disk (C:)"];
        for (i, name) in items.iter().enumerate() {
            let ry = body_y + 14 + i * 30;
            let active_here = (name == &"Disk (C:)" && path.starts_with("C:"))
                || (name == &"Home" && path == "/");
            let fg = if active_here { ACCENT } else { TEXT_LIGHT };
            w.draw_text_at(sidebar_x + 12, ry + 5, name, fg, SURFACE_DARK_2);
        }

        // ---- File view ----
        let view_x = x + EXP_SIDEBAR_W + 20;
        let view_y = body_y;
        let view_w = ww.saturating_sub(EXP_SIDEBAR_W + 30);
        let view_h = body_h;
        let view_bg = Color { r: 37, g: 40, b: 48 };
        w.fill_round_rect(view_x, view_y, view_w, view_h, 6, view_bg);

        w.draw_text_at(view_x + 14, view_y + 10, "Name", MUTED, view_bg);
        w.draw_text_at(view_x + view_w.saturating_sub(100), view_y + 10, "Size", MUTED, view_bg);

        // Получаем список — либо из FAT, либо из RAMFS.
        let entries: Vec<(String, bool, u32)> = if path.starts_with("C:") {
            let mut guard = DISK.lock();
            if let Some(fat) = guard.as_mut() {
                fat.list_root()
                    .into_iter()
                    .map(|e| (e.name, e.kind == FatKind::Directory, e.size))
                    .collect()
            } else {
                Vec::new()
            }
        } else {
            self.fs
                .list(path)
                .into_iter()
                .map(|e| (e.name, e.kind == NodeKind::Directory, e.size as u32))
                .collect()
        };

        for (i, (name, is_dir, size)) in entries.iter().enumerate() {
            let ry = view_y + 34 + i * EXP_ROW_H;
            if ry + EXP_ROW_H >= view_y + view_h { break; }
            let sel = selected == Some(i);
            let bg = if sel { ACCENT } else { view_bg };
            if sel {
                w.fill_round_rect(view_x + 6, ry, view_w.saturating_sub(12), EXP_ROW_H - 2, 5, bg);
            }
            let icon = if *is_dir { "[D]" } else { "[F]" };
            w.draw_text_at(view_x + 12, ry + 5, icon, TEXT_LIGHT, bg);
            w.draw_text_at(view_x + 48, ry + 5, name, TEXT_LIGHT, bg);
            if !*is_dir {
                let sz = format!("{} B", size);
                w.draw_text_at(view_x + view_w.saturating_sub(100), ry + 5, &sz, MUTED, bg);
            }
        }

        // ---- Dialog ----
        let dlg = match mode {
            ExplorerMode::NewFolder { name } => Some(("New folder name:", name.as_str())),
            ExplorerMode::Rename { name } => Some(("New name:", name.as_str())),
            ExplorerMode::Browse => None,
        };
        if let Some((label, value)) = dlg {
            let dlg_w = 320usize;
            let dlg_h = 120usize;
            let dlg_x = x + (ww.saturating_sub(dlg_w)) / 2;
            let dlg_y = y + (wh.saturating_sub(dlg_h)) / 2;

            w.fill_round_rect(dlg_x + 3, dlg_y + 3, dlg_w, dlg_h, 10, Color { r: 8, g: 10, b: 14 });
            w.fill_round_rect(dlg_x, dlg_y, dlg_w, dlg_h, 10, SURFACE_DARK_2);
            w.draw_text_at(dlg_x + 16, dlg_y + 16, label, TEXT_LIGHT, SURFACE_DARK_2);

            let field_x = dlg_x + 16;
            let field_y = dlg_y + 46;
            let field_w = dlg_w - 32;
            widgets::text_field_modern(w, field_x, field_y, field_w, FIELD_H, value, true);

            w.draw_text_at(
                dlg_x + 16,
                dlg_y + dlg_h - 24,
                "Enter - OK, Esc - Cancel",
                MUTED,
                SURFACE_DARK_2,
            );
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
        widgets::button_modern(w, add_btn_x, field_y, 72, FIELD_H, "Add", ACCENT, Color::WHITE, false);

        let list_x = x + pad;
        let list_y = field_y + FIELD_H + 12;
        let list_w = win.w.saturating_sub(pad * 2);
        let list_h = win.h.saturating_sub(TITLE_H + FIELD_H + 4 * pad + BUTTON_H);
        widgets::list_box_modern(w, list_x, list_y, list_w, list_h, items, selected);

        let remove_btn_y = list_y + list_h + 10;
        widgets::button_modern(
            w,
            list_x,
            remove_btn_y,
            120,
            BUTTON_H,
            "Remove",
            Color { r: 180, g: 60, b: 60 },
            Color::WHITE,
            false,
        );
    }

    fn draw_calc(&self, w: &mut Writer, win: &Window, display: &str) {
        let layout = calc_layout(win);
        let (dx, dy, dw, dh) = (
            layout.display_x,
            layout.display_y,
            layout.display_w,
            layout.display_h,
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
            let bg = if pressed { ACCENT } else { Color { r: 45, g: 48, b: 56 } };
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
            "v0.7", MUTED, SURFACE_DARK_2,
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