//! Обработка ввода: клавиатура, мышь, контекстное меню.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::fs;
use crate::interrupts::ticks;
use crate::keyboard::Key;
use crate::mouse;
use crate::widgets::{self, BUTTON_H, FIELD_H, ROW_H};

use super::anim::RectOnDone;
use super::chrome::calc_layout;
use super::notepad;
use super::state::*;
use super::theme;
use super::Wm;

/// Окно «двойного клика» в тиках (~0.33 сек).
const DOUBLE_CLICK_TICKS: u64 = 6;

impl Wm {
    pub(in crate::gui) fn on_move(&mut self, x: i32, y: i32) {
        self.hover = (x, y);

        if self.drag.is_some() {
            let (sw, _sh) = mouse::screen_size();
            self.snap_zone = if x < 20 {
                Some(SnapZone::Left)
            } else if x > sw - 20 {
                Some(SnapZone::Right)
            } else if y < 10 {
                Some(SnapZone::Top)
            } else {
                None
            };
        } else {
            self.snap_zone = None;
        }

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

        let active = self.active;
        if active < self.windows.len() {
            let drag_scroll = matches!(
                &self.windows[active].content,
                App::Explorer { scroll_drag: true, .. }
            );
            if drag_scroll {
                self.update_scroll_from_mouse(active, y);
            }
        }
    }

    pub(in crate::gui) fn on_wheel(&mut self, delta: i32) {
        let idx = self.active;
        if idx >= self.windows.len() { return; }
        if self.windows[idx].minimized { return; }
        const STEP: usize = 3;
        if let App::Explorer { scroll, .. } = &mut self.windows[idx].content {
            let amount = delta.unsigned_abs() as usize * STEP;
            *scroll = if delta > 0 { scroll.saturating_sub(amount) } else { scroll.saturating_add(amount) };
        }
    }

    fn update_scroll_from_mouse(&mut self, idx: usize, my: i32) {
        let win = &self.windows[idx];
        let wy = win.y.max(0) as usize;
        let toolbar_y = wy + TITLE_H + 8;
        let body_y = toolbar_y + EXP_TOOLBAR_H + 8;
        let body_h = (wy + win.h).saturating_sub(body_y + 12);

        let p = match &self.windows[idx].content {
            App::Explorer { path, .. } => path.clone(),
            _ => return,
        };
        let total = self.list_dir_entries(&p).len();
        let visible_rows = body_h / EXP_ROW_H;
        let max_scroll = total.saturating_sub(visible_rows);

        if max_scroll == 0 {
            if let App::Explorer { scroll, .. } = &mut self.windows[idx].content { *scroll = 0; }
            return;
        }
        let rel = (my.max(0) as usize).saturating_sub(body_y);
        let fraction = rel as f32 / body_h.max(1) as f32;
        let new_scroll = (fraction * max_scroll as f32) as usize;
        if let App::Explorer { scroll, .. } = &mut self.windows[idx].content {
            *scroll = new_scroll.min(max_scroll);
        }
        self.dirty = true;
    }

    pub(in crate::gui) fn on_right_down(&mut self, x: i32, y: i32) {
        self.close_all_ctx_menus();
        let (_, sh) = mouse::screen_size();
        let taskbar_y = (sh as usize).saturating_sub(TASKBAR_H);
        if (y as usize) >= taskbar_y { return; }

        let mut hit_idx = None;
        for idx in (0..self.windows.len()).rev() {
            if self.windows[idx].minimized { continue; }
            let w = &self.windows[idx];
            if widgets::hit(x, y, w.x as usize, w.y as usize, w.w, w.h) {
                hit_idx = Some(idx);
                break;
            }
        }
        let Some(idx) = hit_idx else { return };
        self.focus_window(idx);

        let is_explorer = matches!(&self.windows[idx].content, App::Explorer { .. });
        if !is_explorer { self.dirty = true; return; }

        let win = &self.windows[self.active];
        let wx = win.x.max(0) as usize;
        let wy = win.y.max(0) as usize;
        let ww = win.w;

        let toolbar_y = wy + TITLE_H + 8;
        let body_y = toolbar_y + EXP_TOOLBAR_H + 8;
        let view_x = wx + EXP_SIDEBAR_W + 20;
        let view_y = body_y;
        let view_w = ww.saturating_sub(EXP_SIDEBAR_W + 30);

        let row = if (x as usize) >= view_x && (x as usize) < view_x + view_w && (y as usize) >= view_y + 34 {
            let scroll = match &self.windows[self.active].content { App::Explorer { scroll, .. } => *scroll, _ => 0 };
            let rel_y = (y as usize) - (view_y + 34);
            Some(scroll + rel_y / EXP_ROW_H)
        } else { None };

        if let Some(row) = row {
            let p = match &self.windows[self.active].content { App::Explorer { path, .. } => path.clone(), _ => return };
            let entries = self.list_dir_entries(&p);
            if row < entries.len() {
                if let App::Explorer { selected, anchor, last_click, ctx_menu, .. } = &mut self.windows[self.active].content {
                    if !selected.contains(&row) { selected.clear(); selected.push(row); }
                    *anchor = Some(row);
                    *last_click = None;
                    *ctx_menu = Some((x, y, row));
                }
                self.dirty = true;
            }
        } else if let App::Explorer { ctx_menu, .. } = &mut self.windows[self.active].content {
            *ctx_menu = Some((x, y, usize::MAX));
            self.dirty = true;
        }
    }

    /// Свернуть окно `idx` с анимацией.
    ///
    /// Внимание: сначала вычитываем все поля окна в локальные переменные, и
    /// только потом мутируем `self.windows[idx]` — иначе borrow checker
    /// ругается на одновременный `&` и `&mut self.windows` (E0502).
    fn minimize_window(&mut self, idx: usize) {
        if idx >= self.windows.len() { return; }

        let (from_x, from_y, from_w, from_h) = {
            let win = &self.windows[idx];
            (win.x, win.y, win.w, win.h)
        };
        let from = (from_x, from_y, from_w, from_h);

        self.windows[idx].restore_rect = Some(from);

        let (_, sh) = mouse::screen_size();
        let to = (from_x, sh + 20, from_w, from_h);
        self.anim.start_rect(idx, from, to, RectOnDone::Minimize);
    }

    /// Развернуть окно `idx` с анимацией.
    fn restore_window(&mut self, idx: usize) {
        if idx >= self.windows.len() { return; }
        let to = self.windows[idx].restore_rect.take().unwrap_or_else(|| {
            let off = idx as i32 * 28;
            (100 + off, 70 + off, self.windows[idx].w, self.windows[idx].h)
        });
        let from = (to.0, mouse::screen_size().1 + 20, to.2, to.3);
        self.windows[idx].minimized = false;
        self.anim.start_rect(idx, from, to, RectOnDone::None);
        self.focus_window(idx);
    }

    /// Максимизировать/восстановить окно `idx`.
    fn toggle_maximize(&mut self, idx: usize) {
        if idx >= self.windows.len() { return; }
        let (sw, sh) = mouse::screen_size();
        let taskbar_h = TASKBAR_H as i32;
        if self.windows[idx].restore_rect.is_some() {
            // Восстановить.
            let to = self.windows[idx].restore_rect.take().unwrap();
            let win = &self.windows[idx];
            let from = (win.x, win.y, win.w, win.h);
            self.anim.start_rect(idx, from, to, RectOnDone::None);
        } else {
            // Максимизировать.
            let win = &self.windows[idx];
            let from = (win.x, win.y, win.w, win.h);
            self.windows[idx].restore_rect = Some(from);
            let to = (0, 0, sw as usize, (sh - taskbar_h) as usize);
            self.anim.start_rect(idx, from, to, RectOnDone::None);
        }
        self.dirty = true;
    }

    pub(in crate::gui) fn on_left_down(&mut self, x: i32, y: i32) {
        self.dirty = true;

        let (_sw, sh) = mouse::screen_size();
        let taskbar_y = (sh as usize).saturating_sub(TASKBAR_H);

        if self.handle_ctx_menu_click(x, y) { return; }

        if self.start_pressed {
            let menu_w = 240usize;
            let menu_h = START_MENU_H;
            let menu_x = 6usize;
            let menu_y = taskbar_y.saturating_sub(menu_h + 6);

            if (x as usize) >= menu_x && (x as usize) < menu_x + menu_w
                && (y as usize) >= menu_y && (y as usize) < menu_y + menu_h
            {
                for (i, item) in START_MENU_ITEMS.iter().enumerate() {
                    let iy = menu_y + 60 + i * 32;
                    if (y as usize) >= iy && (y as usize) < iy + 28 {
                        match *item {
                            "Программы" => self.open_app(AppKind::Explorer),
                            "Документы" => {
                                self.open_app(AppKind::Explorer);
                                for w in self.windows.iter_mut().rev() {
                                    if let App::Explorer { path, selected, anchor, .. } = &mut w.content {
                                        *path = "/Documents".to_string();
                                        selected.clear();
                                        *anchor = None;
                                        break;
                                    }
                                }
                            }
                            "Сменить тему" => {
                                theme::toggle();
                                self.anim.toast("Тема изменена", super::anim::ToastKind::Info);
                            }
                            "О системе" => {
                                let text = "Rust OS v0.9\n\nRust OS — учебная ОС на Rust.\n";
                                self.spawn_app(AppKind::Notepad, Some(("О системе.txt".to_string(), text.to_string())));
                            }
                            "Перезагрузка" => crate::power::reboot(),
                            "Выключение" => crate::power::shutdown(),
                            _ => {}
                        }
                        self.start_pressed = false;
                        self.dirty = true;
                        return;
                    }
                }
                self.start_pressed = false;
                self.dirty = true;
                return;
            }
            self.start_pressed = false;
            self.dirty = true;
        }

        // Taskbar.
        if (y as usize) >= taskbar_y {
            if x < 78 { self.start_pressed = !self.start_pressed; return; }
            let mut bx = 86;
            for idx in 0..self.windows.len() {
                let bw = crate::framebuffer::Writer::text_width(&self.windows[idx].title) + 24;
                if (x as usize) >= bx && (x as usize) < bx + bw {
                    if idx == self.active && !self.windows[idx].minimized {
                        self.minimize_window(idx);
                    } else if self.windows[idx].minimized {
                        self.restore_window(idx);
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
                let dbl = matches!(self.last_click, Some((t, k)) if k == *kind && now.saturating_sub(t) < 30);
                if dbl { self.open_app(*kind); self.last_click = None; }
                else { self.last_click = Some((now, *kind)); }
                return;
            }
        }

        if let Some((id, lx, ly)) = crate::win::hit_test(x, y) {
            if let Some(pid) = crate::win::pid_of(id) {
                crate::win::push_event_to_pid(pid, crate::win::WinEvent {
                    kind: crate::win::EV_MOUSE_DOWN,
                    x: lx,
                    y: ly,
                    code: 1,
                });
                self.dirty = true;
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
            let (wx, wy, ww) = { let w = &self.windows[self.active]; (w.x, w.y, w.w) };
            let rel_x = (x - wx) as usize;
            let rel_y = (y - wy) as usize;

            if rel_y < TITLE_H {
                let cb_x = ww.saturating_sub(8 + CLOSE_BTN_W);
                let mb_x = cb_x.saturating_sub(6 + MIN_BTN_W);
                if rel_x >= cb_x {
                    self.close_active();
                } else if rel_x >= mb_x {
                    let i = self.active;
                    self.minimize_window(i);
                } else {
                    // Двойной клик на заголовке = maximize/restore.
                    let now = ticks();
                    let dbl = matches!(
                        self.last_title_click,
                        Some((t, i)) if i == idx && now.saturating_sub(t) < DOUBLE_CLICK_TICKS
                    );
                    if dbl {
                        self.last_title_click = None;
                        self.toggle_maximize(idx);
                    } else {
                        self.last_title_click = Some((now, idx));
                        self.drag = Some(Drag { idx: self.active, ox: x - wx, oy: y - wy });
                    }
                }
            } else {
                self.handle_content_click(x, y);
            }
        }
    }

    pub(in crate::gui) fn on_button_up(&mut self, x: i32, y: i32) {
        if self.drag.is_some() {
            if let Some(zone) = self.snap_zone.take() {
                let (sw, sh) = mouse::screen_size();
                let idx = self.active;
                if idx < self.windows.len() {
                    let taskbar_h = TASKBAR_H as i32;
                    let win = &self.windows[idx];
                    let from = (win.x, win.y, win.w, win.h);
                    let to = match zone {
                        SnapZone::Left => (
                            0, 0,
                            (sw / 2) as usize,
                            (sh - taskbar_h) as usize,
                        ),
                        SnapZone::Right => (
                            sw / 2, 0,
                            (sw / 2) as usize,
                            (sh - taskbar_h) as usize,
                        ),
                        SnapZone::Top => (
                            0, 0,
                            sw as usize,
                            (sh - taskbar_h) as usize,
                        ),
                    };
                    self.anim.start_rect(idx, from, to, RectOnDone::None);
                }
            }
        }
        self.snap_zone = None;

        if let Some((id, lx, ly)) = crate::win::hit_test(x, y) {
            if let Some(pid) = crate::win::pid_of(id) {
                crate::win::push_event_to_pid(pid, crate::win::WinEvent {
                    kind: crate::win::EV_MOUSE_UP,
                    x: lx,
                    y: ly,
                    code: 1,
                });
            }
        }
        self.drag = None;
        self.painting = None;
        let active = self.active;
        if active < self.windows.len() {
            if let App::Explorer { scroll_drag, .. } = &mut self.windows[active].content {
                *scroll_drag = false;
            }
        }
        self.dirty = true;
    }

    fn handle_ctx_menu_click(&mut self, mx: i32, my: i32) -> bool {
        let active = self.active;
        if active >= self.windows.len() { return false; }
        let ctx = match &self.windows[active].content { App::Explorer { ctx_menu, .. } => *ctx_menu, _ => None };
        let Some((cx, cy, row)) = ctx else { return false };

        let is_item = row != usize::MAX;
        let items: &[&str] = if is_item { &["Открыть", "Копировать", "Удалить"] } else { &["Вставить", "Новая папка", "Новый файл"] };
        let menu_h = items.len() * CTX_ITEM_H + 8;
        let in_menu = (mx as usize) >= cx as usize && (mx as usize) < cx as usize + CTX_MENU_W
            && (my as usize) >= cy as usize && (my as usize) < cy as usize + menu_h;

        if !in_menu {
            if let App::Explorer { ctx_menu, .. } = &mut self.windows[active].content { *ctx_menu = None; }
            self.dirty = true;
            return false;
        }

        let rel_y = (my as usize).saturating_sub(cy as usize + 4);
        let item_idx = rel_y / CTX_ITEM_H;
        if item_idx >= items.len() {
            if let App::Explorer { ctx_menu, .. } = &mut self.windows[active].content { *ctx_menu = None; }
            self.dirty = true;
            return true;
        }
        let item = items[item_idx];
        let (p, on_disk) = match &self.windows[active].content {
            App::Explorer { path, .. } => (path.clone(), path.starts_with("C:")),
            _ => return true,
        };

        match item {
            "Открыть" => {
                let entries = self.list_dir_entries(&p);
                if row < entries.len() {
                    let (name, is_dir, _) = entries[row].clone();
                    if is_dir && !on_disk {
                        let full = fs::join(&p, &name);
                        if let App::Explorer { path, history, selected, anchor, scroll, .. } = &mut self.windows[active].content {
                            history.push(path.clone());
                            *path = full;
                            selected.clear();
                            *anchor = None;
                            *scroll = 0;
                        }
                    } else if name.ends_with(".txt") || name.ends_with(".TXT") {
                        if on_disk {
                            if let Some(text) = self.read_disk_file(&name) {
                                self.spawn_app(AppKind::Notepad, Some((format!("C:/{}", name), text)));
                            }
                        } else {
                            let full = fs::join(&p, &name);
                            self.open_notepad_with_file(full);
                        }
                    }
                }
            }
            "Копировать" => self.copy_selection_to_clipboard(active),
            "Удалить" => {
                let entries = self.list_dir_entries(&p);
                if row < entries.len() {
                    if let App::Explorer { mode, .. } = &mut self.windows[active].content {
                        *mode = ExplorerMode::ConfirmDelete { count: 1, on_disk };
                    }
                }
            }
            "Вставить" => self.paste_clipboard(active),
            "Новая папка" => {
                if !on_disk {
                    if let App::Explorer { mode, .. } = &mut self.windows[active].content {
                        *mode = ExplorerMode::NewFolder { name: String::new() };
                    }
                }
            }
            "Новый файл" => {
                if let App::Explorer { mode, .. } = &mut self.windows[active].content {
                    *mode = ExplorerMode::NewFile { name: String::new() };
                }
            }
            _ => {}
        }
        if let App::Explorer { ctx_menu, .. } = &mut self.windows[active].content { *ctx_menu = None; }
        self.dirty = true;
        true
    }

    fn handle_content_click(&mut self, mx: i32, my: i32) {
        let active = self.active;
        let is_explorer = matches!(&self.windows[active].content, App::Explorer { .. });
        if is_explorer { self.handle_explorer_click(mx, my); return; }

        let win = &mut self.windows[active];
        let wx = win.x.max(0) as usize;
        let wy = win.y.max(0) as usize;

        match &mut win.content {
            App::Notepad { text, cursor, selection_anchor, .. } => {
                let pad = 14;
                let field_x = wx + pad;
                let field_y = wy + TITLE_H + 6;
                let px = field_x + 8;
                let py = field_y + 8;
                if mx >= px as i32 && my >= py as i32 {
                    let rel_x = (mx - px as i32) as usize;
                    let rel_y = (my - py as i32) as usize;
                    let line = rel_y / crate::framebuffer::FONT_HEIGHT;
                    let col = rel_x / 9;
                    *cursor = notepad::lc_to_idx(text, line, col);
                    *selection_anchor = None;
                }
            }
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
                    if let Some(sel) = selected.take() { if sel < items.len() { items.remove(sel); } }
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
                if widgets::hit(mx, my, bx, by.saturating_sub(50), bw, 40) { self.calc_input('C'); return; }
                let labels = [["7","8","9","/"],["4","5","6","*"],["1","2","3","-"],["0",".","=","+"]];
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
        let (wx, wy, ww, wh) = { let win = &self.windows[idx]; (win.x.max(0) as usize, win.y.max(0) as usize, win.w, win.h) };
        let toolbar_y = wy + TITLE_H + 8;
        let body_y = toolbar_y + EXP_TOOLBAR_H + 8;
        let back_x = wx + 10;
        let up_x = back_x + 48;
        let nf_x = up_x + 48;
        let fl_x = nf_x + 90;
        let del_x = fl_x + 90;
        let ren_x = del_x + 80;

        let is_dialog_open = matches!(&self.windows[idx].content, App::Explorer { mode, .. } if !matches!(mode, ExplorerMode::Browse));
        if is_dialog_open {
            if let App::Explorer { mode, .. } = &mut self.windows[idx].content { *mode = ExplorerMode::Browse; }
            self.dirty = true;
            return;
        }

        let body_h = (wy + wh).saturating_sub(body_y + 12);
        let view_x = wx + EXP_SIDEBAR_W + 20;
        let view_w = ww.saturating_sub(EXP_SIDEBAR_W + 30);
        let scrollbar_x = view_x + view_w - SCROLLBAR_W - 2;

        if (mx as usize) >= scrollbar_x && (mx as usize) < scrollbar_x + SCROLLBAR_W
            && (my as usize) >= body_y && (my as usize) < body_y + body_h
        {
            if let App::Explorer { scroll_drag, .. } = &mut self.windows[idx].content { *scroll_drag = true; }
            self.update_scroll_from_mouse(idx, my);
            self.dirty = true;
            return;
        }

        if (my as usize) >= toolbar_y && (my as usize) < toolbar_y + EXP_TOOLBAR_H {
            if widgets::hit(mx, my, back_x, toolbar_y, 42, EXP_TOOLBAR_H) {
                if let App::Explorer { path, history, selected, anchor, scroll, .. } = &mut self.windows[idx].content {
                    if let Some(prev) = history.pop() { *path = prev; selected.clear(); *anchor = None; *scroll = 0; }
                }
                self.dirty = true;
                return;
            }
            if widgets::hit(mx, my, up_x, toolbar_y, 42, EXP_TOOLBAR_H) {
                if let App::Explorer { path, history, selected, anchor, scroll, .. } = &mut self.windows[idx].content {
                    let cur = path.clone();
                    if cur != "/" && !cur.starts_with("C:") {
                        let parent = fs::parent_path(&cur);
                        history.push(cur);
                        *path = parent;
                        selected.clear();
                        *anchor = None;
                        *scroll = 0;
                    }
                }
                self.dirty = true;
                return;
            }
            if widgets::hit(mx, my, nf_x, toolbar_y, 86, EXP_TOOLBAR_H) {
                let p = match &self.windows[idx].content { App::Explorer { path, .. } => path.clone(), _ => return };
                if let App::Explorer { mode, .. } = &mut self.windows[idx].content {
                    if p.starts_with("C:") { *mode = ExplorerMode::NewFile { name: String::new() }; }
                    else { *mode = ExplorerMode::NewFolder { name: String::new() }; }
                }
                self.dirty = true;
                return;
            }
            if widgets::hit(mx, my, fl_x, toolbar_y, 86, EXP_TOOLBAR_H) {
                if let App::Explorer { mode, .. } = &mut self.windows[idx].content {
                    *mode = ExplorerMode::NewFile { name: String::new() };
                }
                self.dirty = true;
                return;
            }
            if widgets::hit(mx, my, del_x, toolbar_y, 76, EXP_TOOLBAR_H) {
                let p = match &self.windows[idx].content { App::Explorer { path, .. } => path.clone(), _ => return };
                let sel = match &self.windows[idx].content { App::Explorer { selected, .. } => selected.clone(), _ => Vec::new() };
                if !sel.is_empty() {
                    let on_disk = p.starts_with("C:");
                    if let App::Explorer { mode, .. } = &mut self.windows[idx].content {
                        *mode = ExplorerMode::ConfirmDelete { count: sel.len(), on_disk };
                    }
                }
                self.dirty = true;
                return;
            }
            if widgets::hit(mx, my, ren_x, toolbar_y, 76, EXP_TOOLBAR_H) {
                let p = match &self.windows[idx].content { App::Explorer { path, .. } => path.clone(), _ => return };
                let sel = match &self.windows[idx].content { App::Explorer { selected, .. } => selected.clone(), _ => Vec::new() };
                if let Some(&i) = sel.first() {
                    if p.starts_with("C:") {
                        let entries = crate::vfs::fat32_list_root();
                        if let Some((name, _, _)) = entries.get(i) {
                            if let App::Explorer { mode, .. } = &mut self.windows[idx].content {
                                *mode = ExplorerMode::Rename { name: name.clone() };
                            }
                        }
                    } else {
                        let entries = crate::vfs::ramfs_list_meta(&p);
                        if let Some((name, _, _)) = entries.get(i) {
                            if let App::Explorer { mode, .. } = &mut self.windows[idx].content {
                                *mode = ExplorerMode::Rename { name: name.clone() };
                            }
                        }
                    }
                }
                self.dirty = true;
                return;
            }
            return;
        }

        let sidebar_x = wx + 10;
        let sidebar_w = EXP_SIDEBAR_W;
        if (mx as usize) >= sidebar_x && (mx as usize) < sidebar_x + sidebar_w && (my as usize) >= body_y {
            let items: [(&str, &str); 6] = [
                ("Домой", "/"), ("Рабочий стол", "/Desktop"),
                ("Документы", "/Documents"), ("Загрузки", "/Downloads"),
                ("Система", "/System"), ("Диск (C:)", "C:/"),
            ];
            for (i, (_, p)) in items.iter().enumerate() {
                let ry = body_y + 14 + i * 30;
                if (my as usize) >= ry && (my as usize) < ry + 26 {
                    if let App::Explorer { path, history, selected, anchor, scroll, .. } = &mut self.windows[idx].content {
                        let cur = path.clone();
                        if cur != *p {
                            history.push(cur);
                            *path = p.to_string();
                            selected.clear();
                            *anchor = None;
                            *scroll = 0;
                        }
                    }
                    self.dirty = true;
                    return;
                }
            }
            return;
        }

        let view_y = body_y;
        if (mx as usize) >= view_x && (mx as usize) < view_x + view_w - SCROLLBAR_W - 2 && (my as usize) >= view_y + 34 {
            let scroll = match &self.windows[idx].content { App::Explorer { scroll, .. } => *scroll, _ => 0 };
            let rel_y = (my as usize) - (view_y + 34);
            let row = scroll + rel_y / EXP_ROW_H;
            let p = match &self.windows[idx].content { App::Explorer { path, .. } => path.clone(), _ => return };
            let entries = self.list_dir_entries(&p);

            if row < entries.len() {
                let now = ticks();
                let dbl = if let App::Explorer { last_click, .. } = &self.windows[idx].content {
                    matches!(last_click, Some((t, r)) if *r == row && now.saturating_sub(*t) < 30)
                } else { false };

                let shift = self.mods_shift;
                let ctrl = self.mods_ctrl;

                if let App::Explorer { selected, anchor, last_click, .. } = &mut self.windows[idx].content {
                    if shift {
                        let a = anchor.unwrap_or(row);
                        let (lo, hi) = if a <= row { (a, row) } else { (row, a) };
                        selected.clear();
                        for i in lo..=hi { selected.push(i); }
                    } else if ctrl {
                        if let Some(pos) = selected.iter().position(|&x| x == row) { selected.remove(pos); }
                        else { selected.push(row); }
                        *anchor = Some(row);
                    } else {
                        selected.clear();
                        selected.push(row);
                        *anchor = Some(row);
                    }
                    *last_click = Some((now, row));
                }

                if dbl {
                    let (name, is_dir, _) = entries[row].clone();
                    if is_dir {
                        if !p.starts_with("C:") {
                            let full = fs::join(&p, &name);
                            if let App::Explorer { path, history, selected, anchor, scroll, last_click, .. } = &mut self.windows[idx].content {
                                history.push(path.clone());
                                *path = full;
                                selected.clear();
                                *anchor = None;
                                *scroll = 0;
                                *last_click = None;
                            }
                        }
                    } else if name.ends_with(".txt") || name.ends_with(".TXT") {
                        if p.starts_with("C:") {
                            if let Some(text) = self.read_disk_file(&name) {
                                self.spawn_app(AppKind::Notepad, Some((format!("C:/{}", name), text)));
                            }
                        } else {
                            let full = fs::join(&p, &name);
                            self.open_notepad_with_file(full);
                        }
                    }
                }
                self.dirty = true;
            } else if !self.mods_ctrl {
                if let App::Explorer { selected, anchor, .. } = &mut self.windows[idx].content {
                    selected.clear();
                    *anchor = None;
                }
                self.dirty = true;
            }
        }
    }

    pub(in crate::gui) fn on_key(&mut self, k: Key, ctrl: bool, alt: bool) {
        if alt && k == Key::Tab {
            if self.windows.is_empty() { return; }
            if !self.switcher_open {
                self.switcher_open = true;
                self.switcher_selected = if self.windows.len() > 1 {
                    (self.active + 1) % self.windows.len()
                } else {
                    self.active
                };
            } else {
                let n = self.windows.len();
                self.switcher_selected = (self.switcher_selected + 1) % n;
            }
            self.dirty = true;
            return;
        }
        if alt && k == Key::F(4) && !self.windows.is_empty() {
            self.close_active();
            return;
        }
        if k == Key::Escape {
            let idx = self.active;
            if let App::Explorer { mode, ctx_menu, .. } = &mut self.windows[idx].content {
                if !matches!(mode, ExplorerMode::Browse) { *mode = ExplorerMode::Browse; self.dirty = true; return; }
                if ctx_menu.is_some() { *ctx_menu = None; self.dirty = true; return; }
            }
            if let App::Notepad { mode, .. } = &mut self.windows[idx].content {
                if !matches!(mode, NotepadMode::Browse) { *mode = NotepadMode::Browse; self.dirty = true; return; }
            }
            self.start_pressed = false;
            self.dirty = true;
            return;
        }

        if self.windows.is_empty() { return; }
        let idx = self.active;

        let is_confirm = matches!(&self.windows[idx].content,
            App::Explorer { mode: ExplorerMode::ConfirmDelete { .. }, .. });
        if is_confirm {
            match k {
                Key::Enter => {
                    self.delete_selected(idx);
                    if let App::Explorer { mode, .. } = &mut self.windows[idx].content { *mode = ExplorerMode::Browse; }
                    self.dirty = true;
                    return;
                }
                Key::Escape => {
                    if let App::Explorer { mode, .. } = &mut self.windows[idx].content { *mode = ExplorerMode::Browse; }
                    self.dirty = true;
                    return;
                }
                _ => {}
            }
        }

        let total_items = match &self.windows[idx].content {
            App::Explorer { path, .. } => self.list_dir_entries(path).len(),
            _ => 0,
        };

        let win = &mut self.windows[idx];
        win.minimized = false;
        self.dirty = true;

        match &mut win.content {
            App::Notepad { text, file, modified, mode, cursor, selection_anchor } => match mode {
                NotepadMode::SaveAs { name } => match k {
                    Key::Enter => {
                        let n = name.clone();
                        if !n.is_empty() {
                            if crate::vfs::fat32_write_file(&n, text.as_bytes()) {
                                *file = Some(format!("C:/{}", n));
                                *modified = false;
                                *mode = NotepadMode::Browse;
                            } else if crate::vfs::ramfs_create_file(
                                "/Documents", &n, text.as_bytes(),
                            ) {
                                *file = Some(format!("/Documents/{}", n));
                                *modified = false;
                                *mode = NotepadMode::Browse;
                            }
                            self.anim.toast("Файл сохранён", super::anim::ToastKind::Success);
                        }
                    }
                    Key::Backspace => { name.pop(); }
                    Key::Char(c) => name.push(c as char),
                    _ => {}
                },
                NotepadMode::Open { name } => match k {
                    Key::Enter => {
                        let n = name.clone();
                        if !n.is_empty() {
                            let mut opened = false;
                            if let Some(data) = crate::vfs::fat32_read_file(&n) {
                                *text = String::from_utf8_lossy(&data).to_string();
                                *file = Some(format!("C:/{}", n));
                                *modified = false;
                                *cursor = 0;
                                *selection_anchor = None;
                                opened = true;
                            }
                            if !opened {
                                let path = format!("/Documents/{}", n);
                                if let Some(v) = crate::vfs::ramfs_read(&path) {
                                    *text = String::from_utf8_lossy(&v).to_string();
                                    *file = Some(path);
                                    *modified = false;
                                    *cursor = 0;
                                    *selection_anchor = None;
                                }
                            }
                            *mode = NotepadMode::Browse;
                        }
                    }
                    Key::Backspace => { name.pop(); }
                    Key::Char(c) => name.push(c as char),
                    _ => {}
                },
                NotepadMode::Browse => {
                    if ctrl && matches!(k, Key::Char(b's') | Key::Char(b'S')) {
                        if let Some(path) = file.clone() {
                            if let Some(name) = path.strip_prefix("C:/") {
                                if crate::vfs::fat32_write_file(name, text.as_bytes()) {
                                    *modified = false;
                                    self.anim.toast("Сохранено", super::anim::ToastKind::Success);
                                }
                            } else if crate::vfs::ramfs_write(&path, text.as_bytes()) {
                                *modified = false;
                                self.anim.toast("Сохранено", super::anim::ToastKind::Success);
                            }
                        } else {
                            *mode = NotepadMode::SaveAs { name: String::new() };
                        }
                        return;
                    }
                    if ctrl && matches!(k, Key::Char(b'o') | Key::Char(b'O')) {
                        *mode = NotepadMode::Open { name: String::new() };
                        return;
                    }
                    if ctrl && matches!(k, Key::Char(b'a') | Key::Char(b'A')) {
                        notepad::select_all(text, cursor, selection_anchor);
                        return;
                    }
                    if ctrl && matches!(k, Key::Char(b'c') | Key::Char(b'C')) {
                        if let Some(s) = notepad::selected_text(text, *cursor, *selection_anchor) {
                            notepad::clipboard_set(s);
                        }
                        return;
                    }
                    if ctrl && matches!(k, Key::Char(b'x') | Key::Char(b'X')) {
                        if let Some(s) = notepad::selected_text(text, *cursor, *selection_anchor) {
                            notepad::clipboard_set(s);
                            notepad::delete_selection(text, cursor, selection_anchor);
                            *modified = true;
                        }
                        return;
                    }
                    if ctrl && matches!(k, Key::Char(b'v') | Key::Char(b'V')) {
                        if !notepad::clipboard_is_empty() {
                            let clip = notepad::clipboard_get();
                            notepad::insert_str(text, cursor, selection_anchor, &clip);
                            *modified = true;
                        }
                        return;
                    }
                    let extend = self.mods_shift;
                    match k {
                        Key::Left if ctrl => notepad::move_cursor(text, cursor, selection_anchor, Direction::WordLeft, extend),
                        Key::Right if ctrl => notepad::move_cursor(text, cursor, selection_anchor, Direction::WordRight, extend),
                        Key::Left => notepad::move_cursor(text, cursor, selection_anchor, Direction::Left, extend),
                        Key::Right => notepad::move_cursor(text, cursor, selection_anchor, Direction::Right, extend),
                        Key::Up => notepad::move_cursor(text, cursor, selection_anchor, Direction::Up, extend),
                        Key::Down => notepad::move_cursor(text, cursor, selection_anchor, Direction::Down, extend),
                        Key::Home if ctrl => notepad::move_cursor(text, cursor, selection_anchor, Direction::DocStart, extend),
                        Key::End if ctrl => notepad::move_cursor(text, cursor, selection_anchor, Direction::DocEnd, extend),
                        Key::Home => notepad::move_cursor(text, cursor, selection_anchor, Direction::Home, extend),
                        Key::End => notepad::move_cursor(text, cursor, selection_anchor, Direction::End, extend),
                        Key::PageUp => notepad::move_cursor(text, cursor, selection_anchor, Direction::PageUp, extend),
                        Key::PageDown => notepad::move_cursor(text, cursor, selection_anchor, Direction::PageDown, extend),
                        Key::Delete => { notepad::delete_forward(text, cursor, selection_anchor); *modified = true; }
                        Key::Backspace => { notepad::delete_backward(text, cursor, selection_anchor); *modified = true; }
                        Key::Enter => { notepad::insert_char(text, cursor, selection_anchor, '\n'); *modified = true; }
                        Key::Tab => { notepad::insert_str(text, cursor, selection_anchor, notepad::TAB_STR); *modified = true; }
                        Key::Char(c) => { notepad::insert_char(text, cursor, selection_anchor, c as char); *modified = true; }
                        _ => {}
                    }
                }
            },
            App::Explorer { path, selected, anchor, history, mode, ctx_menu, scroll, .. } => {
                if ctx_menu.is_some() { if k == Key::Escape { *ctx_menu = None; } return; }
                match mode {
                    ExplorerMode::ConfirmDelete { .. } => {}
                    ExplorerMode::NewFolder { name } => match k {
                        Key::Enter => {
                            let n = name.clone();
                            let p = path.clone();
                            let ok = if p.starts_with("C:") {
                                crate::vfs::fat32_mkdir(&n)
                            } else {
                                crate::vfs::ramfs_mkdir(&p, &n)
                            };
                            if ok {
                                *mode = ExplorerMode::Browse;
                                self.anim.toast("Папка создана", super::anim::ToastKind::Success);
                            }
                        }
                        Key::Backspace => { name.pop(); }
                        Key::Char(c) => name.push(c as char),
                        _ => {}
                    },
                    ExplorerMode::NewFile { name } => match k {
                        Key::Enter => {
                            let n = name.clone();
                            let p = path.clone();
                            if p.starts_with("C:") {
                                if crate::vfs::fat32_create_file(&n) {
                                    *mode = ExplorerMode::Browse;
                                    self.anim.toast("Файл создан", super::anim::ToastKind::Success);
                                }
                            } else if crate::vfs::ramfs_create_file(&p, &n, b"") {
                                *mode = ExplorerMode::Browse;
                                self.anim.toast("Файл создан", super::anim::ToastKind::Success);
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
                            let sel = if selected.is_empty() { None } else { Some(selected[0]) };
                            if let Some(i) = sel {
                                if p.starts_with("C:") {
                                    let entries = crate::vfs::fat32_list_root();
                                    if let Some((old, _, _)) = entries.get(i) {
                                        if crate::vfs::fat32_rename(old, &n) {
                                            *mode = ExplorerMode::Browse;
                                            self.anim.toast("Переименовано", super::anim::ToastKind::Success);
                                        }
                                    }
                                } else {
                                    let entries = crate::vfs::ramfs_list_meta(&p);
                                    if let Some((old, _, _)) = entries.get(i) {
                                        let full = fs::join(&p, old);
                                        if crate::vfs::ramfs_rename(&full, &n) {
                                            *mode = ExplorerMode::Browse;
                                            self.anim.toast("Переименовано", super::anim::ToastKind::Success);
                                        }
                                    }
                                }
                            }
                        }
                        Key::Backspace => { name.pop(); }
                        Key::Char(c) => name.push(c as char),
                        _ => {}
                    },
                    ExplorerMode::Browse => match k {
                        Key::Backspace => {
                            let cur = path.clone();
                            if cur != "/" && !cur.starts_with("C:") {
                                let parent = fs::parent_path(&cur);
                                history.push(cur);
                                *path = parent;
                                selected.clear();
                                *anchor = None;
                                *scroll = 0;
                            }
                        }
                        Key::Enter => {
                            if let Some(&i) = selected.first() {
                                if !path.starts_with("C:") {
                                    let entries = crate::vfs::ramfs_list_meta(path);
                                    if let Some((name, is_dir, _)) = entries.get(i) {
                                        if *is_dir {
                                            let full = fs::join(path, name);
                                            history.push(path.clone());
                                            *path = full;
                                            selected.clear();
                                            *anchor = None;
                                            *scroll = 0;
                                        }
                                    }
                                }
                            }
                        }
                        Key::Delete => {
                            if !selected.is_empty() {
                                let on_disk = path.starts_with("C:");
                                *mode = ExplorerMode::ConfirmDelete { count: selected.len(), on_disk };
                            }
                        }
                        Key::F(2) => {
                            let p = path.clone();
                            if let Some(&i) = selected.first() {
                                if p.starts_with("C:") {
                                    let entries = crate::vfs::fat32_list_root();
                                    if let Some((name, _, _)) = entries.get(i) {
                                        *mode = ExplorerMode::Rename { name: name.clone() };
                                    }
                                } else {
                                    let entries = crate::vfs::ramfs_list_meta(&p);
                                    if let Some((name, _, _)) = entries.get(i) {
                                        *mode = ExplorerMode::Rename { name: name.clone() };
                                    }
                                }
                            }
                        }
                        Key::Char(c) if ctrl && (c == b'n' || c == b'N') => {
                            let p = path.clone();
                            if p.starts_with("C:") { *mode = ExplorerMode::NewFile { name: String::new() }; }
                            else { *mode = ExplorerMode::NewFolder { name: String::new() }; }
                        }
                        Key::Char(c) if ctrl && (c == b'c' || c == b'C') => self.copy_selection_to_clipboard(idx),
                        Key::Char(c) if ctrl && (c == b'v' || c == b'V') => self.paste_clipboard(idx),
                        Key::Char(c) if ctrl && (c == b'a' || c == b'A') => {
                            selected.clear();
                            for i in 0..total_items { selected.push(i); }
                            *anchor = Some(0);
                        }
                        Key::Up => {
                            if let Some(&cur) = selected.first() {
                                if cur > 0 {
                                    selected.clear();
                                    selected.push(cur - 1);
                                    *anchor = Some(cur - 1);
                                    if cur - 1 < *scroll { *scroll = cur - 1; }
                                }
                            }
                        }
                        Key::Down => {
                            if let Some(&cur) = selected.first() {
                                if cur + 1 < total_items {
                                    selected.clear();
                                    selected.push(cur + 1);
                                    *anchor = Some(cur + 1);
                                    let visible = 10usize;
                                    if cur + 1 >= *scroll + visible { *scroll = cur + 1 - visible + 1; }
                                }
                            }
                        }
                        Key::PageUp => { *scroll = scroll.saturating_sub(10); }
                        Key::PageDown => { *scroll += 10; }
                        Key::Home => { *scroll = 0; }
                        Key::End => { *scroll = 9999; }
                        _ => {}
                    },
                }
            }
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
                *display = if (r - rounded as f64).abs() < 0.0001 { format!("{}", rounded) } else { format!("{:.4}", r) };
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
            let steps = ((px as i32 - px0 as i32).abs()).max((py as i32 - py0 as i32).abs()).max(1);
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
}