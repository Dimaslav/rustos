//! Оконный менеджер: состояние, диспетчер, оконный менеджмент.

pub mod anim;
pub mod chrome;
pub mod draw;
pub mod icons;
pub mod input;
pub mod notepad;
pub mod state;
pub mod theme;

pub use state::{App, AppKind, Window, CURSOR};

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::framebuffer;
use crate::fs;
use crate::interrupts::{ticks, TIMER_HZ};
use crate::keyboard;
use crate::mouse::{self, MouseButton, MouseEvent};
use crate::sound;

pub struct WallpaperBuf {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

pub struct Wm {
    pub(in crate::gui) windows: Vec<Window>,
    pub(in crate::gui) active: usize,
    pub(in crate::gui) clipboard: Vec<state::ClipboardItem>,
    pub(in crate::gui) clipboard_from: String,
    pub(in crate::gui) drag: Option<state::Drag>,
    pub(in crate::gui) painting: Option<usize>,
    pub(in crate::gui) start_pressed: bool,
    pub(in crate::gui) last_click: Option<(u64, AppKind)>,
    pub(in crate::gui) last_title_click: Option<(u64, usize)>,
    pub(in crate::gui) hover: (i32, i32),
    pub(in crate::gui) dirty: bool,
    pub(in crate::gui) cursor_dirty: bool,
    pub(in crate::gui) mods_shift: bool,
    pub(in crate::gui) mods_ctrl: bool,
    pub(in crate::gui) mods_alt: bool,

    pub(in crate::gui) anim: anim::AnimState,
    pub(in crate::gui) switcher_open: bool,
    pub(in crate::gui) switcher_selected: usize,
    pub(in crate::gui) snap_zone: Option<state::SnapZone>,
    pub(in crate::gui) prev_alt: bool,

    pub(in crate::gui) wallpaper: Option<WallpaperBuf>,
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

        {
            let (shift, ctrl, alt) = keyboard::modifiers();
            wm.mods_shift = shift;
            wm.mods_ctrl = ctrl;
            wm.mods_alt = alt;
        }

        while let Some(ev) = mouse::pop_event() {
            match ev {
                MouseEvent::ButtonDown(MouseButton::Left) => {
                    wm.on_left_down(pos.0, pos.1);
                    wm.dirty = true;
                }
                MouseEvent::ButtonUp(MouseButton::Left) => {
                    wm.on_button_up(pos.0, pos.1);
                    wm.dirty = true;
                }
                MouseEvent::ButtonDown(MouseButton::Right) => {
                    wm.on_right_down(pos.0, pos.1);
                    wm.dirty = true;
                }
                MouseEvent::ButtonUp(MouseButton::Right) => {}
                MouseEvent::Wheel(delta) => {
                    wm.on_wheel(delta);
                    wm.dirty = true;
                }
            }
        }

        if !keyboard::user_owns() {
            while let Some(k) = keyboard::pop() {
                let (_, ctrl, alt) = keyboard::modifiers();
                wm.on_key(k, ctrl, alt);
                wm.dirty = true;
            }
        }

        if wm.switcher_open && !wm.mods_alt {
            wm.switcher_close_and_apply();
            wm.dirty = true;
        }

        // Применяем rect-анимацию к окну + финализируем по завершении.
        let mut finalize: Option<(usize, anim::RectOnDone)> = None;
        if let Some(rc) = &wm.anim.rect_anim {
            let idx = rc.win_idx;
            if idx < wm.windows.len() {
                let (nx, ny, nw, nh) = rc.current();
                wm.windows[idx].x = nx;
                wm.windows[idx].y = ny;
                wm.windows[idx].w = nw;
                wm.windows[idx].h = nh;
                if rc.done() {
                    finalize = Some((idx, rc.on_done));
                }
            } else {
                wm.anim.rect_anim = None;
            }
        }
        if let Some((idx, on_done)) = finalize {
            wm.anim.rect_anim = None;
            if idx < wm.windows.len() && on_done == anim::RectOnDone::Minimize {
                wm.windows[idx].minimized = true;
            }
        }

        if wm.anim.tick() {
            wm.dirty = true;
        }

        crate::sched::reap_finished();
        crate::task::run_ready();

        if crate::interrupts::take_clock_dirty() {
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
            clipboard: Vec::new(),
            clipboard_from: String::new(),
            drag: None,
            painting: None,
            start_pressed: false,
            last_click: None,
            last_title_click: None,
            hover: mouse::position(),
            dirty: true,
            cursor_dirty: false,
            mods_shift: false,
            mods_ctrl: false,
            mods_alt: false,
            anim: anim::AnimState::new(),
            switcher_open: false,
            switcher_selected: 0,
            snap_zone: None,
            prev_alt: false,
            wallpaper: None,
        };
        wm.load_wallpaper();
        wm.open_app(AppKind::Explorer);
        wm
    }

    fn load_wallpaper(&mut self) {
        let names = ["WALLPAPER.BMP", "wallpaper.bmp", "WALLPAPER.bmp"];
        for name in &names {
            if let Some(data) = crate::vfs::fat32_read_file(name) {
                crate::serial_println!("[wallpaper] {} size={}", name, data.len());
                if let Some(bmp) = crate::bmp::decode(&data) {
                    crate::serial_println!(
                        "[wallpaper] decoded {}x{}",
                        bmp.width, bmp.height
                    );
                    self.wallpaper = Some(WallpaperBuf {
                        width: bmp.width,
                        height: bmp.height,
                        pixels: bmp.pixels,
                    });
                    return;
                } else {
                    crate::serial_println!("[wallpaper] decode failed");
                }
            }
        }
        crate::serial_println!("[wallpaper] no wallpaper, using gradient");
    }

    pub(in crate::gui) fn focus_window(&mut self, idx: usize) {
        if idx >= self.windows.len() { return; }
        if idx != self.windows.len() - 1 {
            let win = self.windows.remove(idx);
            self.windows.push(win);
            for a in self.anim.window_anims.iter_mut() {
                if a.win_idx == idx {
                    a.win_idx = self.windows.len() - 1;
                } else if a.win_idx > idx {
                    a.win_idx -= 1;
                }
            }
            if let Some(r) = &mut self.anim.rect_anim {
                if r.win_idx == idx {
                    r.win_idx = self.windows.len() - 1;
                } else if r.win_idx > idx {
                    r.win_idx -= 1;
                }
            }
        }
        self.active = self.windows.len() - 1;
        self.windows[self.active].minimized = false;
        self.dirty = true;
    }

    pub(in crate::gui) fn close_active(&mut self) {
        if self.windows.is_empty() { return; }
        self.windows.remove(self.active);
        self.anim.window_anims.retain(|a| a.win_idx != self.active);
        for a in self.anim.window_anims.iter_mut() {
            if a.win_idx > self.active { a.win_idx -= 1; }
        }
        if let Some(r) = &mut self.anim.rect_anim {
            if r.win_idx == self.active {
                self.anim.rect_anim = None;
            } else if r.win_idx > self.active {
                r.win_idx -= 1;
            }
        }
        if self.windows.is_empty() {
            self.active = 0;
        } else if self.active >= self.windows.len() {
            self.active = self.windows.len() - 1;
        }
        self.dirty = true;
    }

    pub(in crate::gui) fn open_app(&mut self, kind: AppKind) {
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

    pub(in crate::gui) fn open_notepad_with_file(&mut self, path: String) {
        let content = crate::vfs::ramfs_read(&path)
            .map(|v| String::from_utf8_lossy(&v).to_string())
            .unwrap_or_default();
        self.spawn_app(AppKind::Notepad, Some((path, content)));
    }

    pub(in crate::gui) fn spawn_app(
        &mut self,
        kind: AppKind,
        notepad_data: Option<(String, String)>,
    ) {
        use alloc::vec;
        use state::{ExplorerMode, NotepadMode};

        let (title, content): (String, App) = match kind {
            AppKind::Explorer => (
                "Проводник".to_string(),
                App::Explorer {
                    path: "/".to_string(),
                    selected: Vec::new(),
                    anchor: None,
                    mode: ExplorerMode::Browse,
                    history: Vec::new(),
                    last_click: None,
                    ctx_menu: None,
                    scroll: 0,
                    scroll_drag: false,
                },
            ),
            AppKind::Notepad => {
                if let Some((file, text)) = notepad_data {
                    let title = file.rsplit('/').next().unwrap_or("Блокнот").to_string();
                    (
                        title,
                        App::Notepad {
                            text,
                            file: Some(file),
                            modified: false,
                            mode: NotepadMode::Browse,
                            cursor: 0,
                            selection_anchor: None,
                        },
                    )
                } else {
                    (
                        "Блокнот".to_string(),
                        App::Notepad {
                            text: String::new(),
                            file: None,
                            modified: false,
                            mode: NotepadMode::Browse,
                            cursor: 0,
                            selection_anchor: None,
                        },
                    )
                }
            }
            AppKind::Todo => (
                "Задачи".to_string(),
                App::Todo {
                    items: vec!["Написать UI".to_string(), "Добавить звук".to_string()],
                    selected: None,
                    input: String::new(),
                },
            ),
            AppKind::Calculator => (
                "Калькулятор".to_string(),
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

        let (ww, wh) = if kind == AppKind::Explorer {
            (720, 460)
        } else {
            (500, 340)
        };
        let off = self.windows.len() as i32 * 28;
        self.windows.push(Window {
            x: 100 + off,
            y: 70 + off,
            w: ww,
            h: wh,
            title,
            content,
            minimized: false,
            restore_rect: None,
        });
        self.active = self.windows.len() - 1;
        self.anim.start_open(self.active);
        self.dirty = true;
        sound::open();
    }

    pub(in crate::gui) fn switcher_close_and_apply(&mut self) {
        if !self.switcher_open { return; }
        let sel = self.switcher_selected;
        self.switcher_open = false;
        if sel < self.windows.len() {
            self.focus_window(sel);
        }
    }

    pub(in crate::gui) fn list_dir_entries(&self, path: &str) -> Vec<(String, bool, u32)> {
        if path.starts_with("C:") {
            crate::vfs::fat32_list_root()
        } else {
            crate::vfs::ramfs_list_meta(path)
                .into_iter()
                .map(|(n, d, s)| (n, d, s as u32))
                .collect()
        }
    }

    pub(in crate::gui) fn read_disk_file(&self, name: &str) -> Option<String> {
        let data = crate::vfs::fat32_read_file(name)?;
        Some(String::from_utf8_lossy(&data).to_string())
    }

    pub(in crate::gui) fn close_all_ctx_menus(&mut self) {
        for w in self.windows.iter_mut() {
            if let App::Explorer { ctx_menu, .. } = &mut w.content {
                *ctx_menu = None;
            }
        }
    }

    pub(in crate::gui) fn copy_selection_to_clipboard(&mut self, active: usize) {
        let (p, on_disk, selected) = match &self.windows[active].content {
            App::Explorer { path, selected, .. } => {
                (path.clone(), path.starts_with("C:"), selected.clone())
            }
            _ => return,
        };
        if selected.is_empty() { return; }

        let mut items: Vec<state::ClipboardItem> = Vec::new();

        if on_disk {
            let entries = crate::vfs::fat32_list_root();
            for &i in &selected {
                if let Some((name, is_dir, _)) = entries.get(i) {
                    if !is_dir {
                        if let Some(data) = crate::vfs::fat32_read_file(name) {
                            items.push(state::ClipboardItem {
                                name: name.clone(),
                                data,
                                is_dir: false,
                            });
                        }
                    }
                }
            }
        } else {
            let entries = crate::vfs::ramfs_list_meta(&p);
            for &i in &selected {
                if let Some((name, is_dir, _)) = entries.get(i) {
                    if !is_dir {
                        let full = fs::join(&p, name);
                        if let Some(data) = crate::vfs::ramfs_read(&full) {
                            items.push(state::ClipboardItem {
                                name: name.clone(),
                                data,
                                is_dir: false,
                            });
                        }
                    }
                }
            }
        }

        if !items.is_empty() {
            crate::serial_println!("[clip] copied {} items from {}", items.len(), p);
            self.clipboard = items;
            self.clipboard_from = p;
            self.anim.toast("Скопировано", anim::ToastKind::Info);
        }
    }

    pub(in crate::gui) fn paste_clipboard(&mut self, active: usize) {
        if self.clipboard.is_empty() { return; }
        let (p, on_disk) = match &self.windows[active].content {
            App::Explorer { path, .. } => (path.clone(), path.starts_with("C:")),
            _ => return,
        };
        let items = self.clipboard.clone();
        let mut pasted = 0usize;
        if on_disk {
            for item in &items {
                if !item.is_dir && crate::vfs::fat32_write_file(&item.name, &item.data) {
                    pasted += 1;
                }
            }
        } else {
            for item in &items {
                if !item.is_dir
                    && crate::vfs::ramfs_create_file(&p, &item.name, &item.data)
                {
                    pasted += 1;
                }
            }
        }
        if pasted > 0 {
            self.anim.toast("Вставлено", anim::ToastKind::Success);
        }
        self.dirty = true;
    }

    pub(in crate::gui) fn delete_selected(&mut self, active: usize) {
        let (p, on_disk, selected) = match &self.windows[active].content {
            App::Explorer { path, selected, .. } => {
                (path.clone(), path.starts_with("C:"), selected.clone())
            }
            _ => return,
        };
        if selected.is_empty() { return; }

        if on_disk {
            let entries = crate::vfs::fat32_list_root();
            let mut sorted = selected.clone();
            sorted.sort();
            sorted.reverse();
            for i in sorted {
                if let Some((name, _, _)) = entries.get(i) {
                    crate::vfs::fat32_remove(name);
                }
            }
        } else {
            let entries = crate::vfs::ramfs_list_meta(&p);
            let mut sorted = selected.clone();
            sorted.sort();
            sorted.reverse();
            for i in sorted {
                if let Some((name, _, _)) = entries.get(i) {
                    let full = fs::join(&p, name);
                    crate::vfs::ramfs_remove(&full);
                }
            }
        }
        if let App::Explorer { selected, anchor, .. } = &mut self.windows[active].content {
            selected.clear();
            *anchor = None;
        }
        self.anim.toast("Удалено", anim::ToastKind::Warn);
        self.dirty = true;
    }

    pub(in crate::gui) fn icons() -> [(AppKind, &'static str, icons::IconKind, framebuffer::Color); 5] {
        use framebuffer::Color;
        use icons::IconKind;
        [
            (AppKind::Explorer,   "Файлы",       IconKind::Folder,   Color { r: 255, g: 195, b: 70  }),
            (AppKind::Notepad,    "Блокнот",     IconKind::Notepad,  Color { r: 100, g: 150, b: 255 }),
            (AppKind::Todo,       "Задачи",      IconKind::Todo,     Color { r: 80,  g: 200, b: 120 }),
            (AppKind::Calculator, "Калькулятор", IconKind::Calc,     Color { r: 240, g: 150, b: 60  }),
            (AppKind::Paint,      "Paint",       IconKind::Paint,    Color { r: 220, g: 90,  b: 180 }),
        ]
    }

    pub(in crate::gui) fn icon_rect(i: usize) -> (usize, usize, usize, usize) {
        use state::{ICON_STEP, ICON_X, ICON_Y};
        let col = i / 4;
        let row = i % 4;
        (ICON_X + col * ICON_STEP, ICON_Y + row * 90, 44, 66)
    }
}