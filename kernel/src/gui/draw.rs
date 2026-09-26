//! Всё рисование. Цвета берутся из `theme::palette()`.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::framebuffer::{Color, Writer, FONT_HEIGHT};
use crate::interrupts::uptime_secs;
use crate::widgets;

use super::anim;
use super::chrome::*;
use super::icons;
use super::state::*;
use super::theme;
use super::Wm;

impl Wm {
    pub(in crate::gui) fn draw(&self, w: &mut Writer) {
        let p = theme::palette();
        if let Some(wp) = &self.wallpaper {
            draw_wallpaper(w, wp);
        } else {
            w.gradient_v(0, 0, w.width, w.height, p.wallpaper_top, p.wallpaper_bottom);
        }

        let icons_arr = Self::icons();
        let selected_kind = self.last_click.map(|(_, k)| k);
        for (i, (kind, label, icon_kind, color)) in icons_arr.iter().enumerate() {
            let (ix, iy, _, _) = Self::icon_rect(i);
            let sel = selected_kind == Some(*kind);
            widgets::desktop_icon_modern(w, ix, iy, label, *icon_kind, *color, sel);
        }

        for (idx, win) in self.windows.iter().enumerate() {
            if win.minimized { continue; }
            let active = idx == self.active;
            self.draw_window(w, win, active);
        }

        x86_64::instructions::interrupts::without_interrupts(|| {
            crate::win::with_windows(|list| {
                for fw in list.iter() {
                    if !fw.alive { continue; }
                    draw_foreign(w, fw);
                }
            });
        });

        for win in self.windows.iter() {
            if win.minimized { continue; }
            if let App::Explorer { ctx_menu: Some((cx, cy, row)), .. } = &win.content {
                self.draw_ctx_menu(w, *cx, *cy, *row);
            }
        }

        self.draw_taskbar(w);

        if self.start_pressed {
            self.draw_start_menu(w);
        }

        if let Some(zone) = self.snap_zone {
            let sw = w.width;
            let sh = w.height.saturating_sub(TASKBAR_H);
            let overlay = Color { r: 90, g: 140, b: 255 };
            let (rx, ry, rw, rh) = match zone {
                SnapZone::Left  => (0, 0, sw / 2, sh),
                SnapZone::Right => (sw / 2, 0, sw / 2, sh),
                SnapZone::Top   => (0, 0, sw, sh),
            };
            for yy in ry..(ry + rh) {
                for xx in rx..(rx + rw) {
                    w.blend_pixel(xx, yy, overlay, 60);
                }
            }
        }

        if self.switcher_open {
            self.draw_switcher(w);
        }

        self.draw_toasts(w);

        // Tooltip — поверх всего, но под курсором (курсор рисуется отдельно).
        if let Some((text, tx, ty)) = &self.tooltip {
            widgets::tooltip(w, (*tx).max(0) as usize, (*ty).max(0) as usize, text);
        }
    }

    fn draw_switcher(&self, w: &mut Writer) {
        let p = theme::palette();
        let n = self.windows.len();
        if n == 0 { return; }

        let card_w = 180usize;
        let card_h = 100usize;
        let gap = 16usize;
        let total_w = n * card_w + (n.saturating_sub(1)) * gap;
        let start_x = (w.width.saturating_sub(total_w)) / 2;
        let y = (w.height.saturating_sub(card_h + 40)) / 2;

        for yy in 0..w.height {
            for xx in 0..w.width {
                w.blend_pixel(xx, yy, Color { r: 0, g: 0, b: 0 }, 90);
            }
        }

        for (i, win) in self.windows.iter().enumerate() {
            let x = start_x + i * (card_w + gap);
            let selected = i == self.switcher_selected;
            let bg = if selected { p.accent } else { p.window_bg_alt };

            w.fill_round_rect_aa(x, y, card_w, card_h, 12, bg);

            let (icon_kind, icon_color) = match &win.content {
                App::Explorer { .. }   => (icons::IconKind::Folder,  Color { r: 255, g: 195, b: 70  }),
                App::Notepad { .. }    => (icons::IconKind::Notepad, Color { r: 100, g: 150, b: 255 }),
                App::Todo { .. }       => (icons::IconKind::Todo,    Color { r: 80,  g: 200, b: 120 }),
                App::Calculator { .. } => (icons::IconKind::Calc,    Color { r: 240, g: 150, b: 60  }),
                App::Paint { .. }      => (icons::IconKind::Paint,   Color { r: 220, g: 90,  b: 180 }),
            };

            w.fill_round_rect_aa(x + card_w / 2 - 20, y + 14, 40, 40, 10, icon_color);
            icons::draw(
                icon_kind, w,
                x + card_w / 2 - 20 + 6,
                y + 14 + 6,
                28,
                Color::WHITE,
            );

            let title_w = Writer::text_width(&win.title);
            let tx = x + (card_w.saturating_sub(title_w)) / 2;
            w.draw_text_at(tx, y + card_h - 26, &win.title, p.text, bg);
        }
    }

    fn draw_toasts(&self, w: &mut Writer) {
        let p = theme::palette();
        let toast_w = 260usize;
        let toast_h = 44usize;
        let gap = 8usize;
        let base_x = w.width.saturating_sub(toast_w + 16) as i32;
        let base_y = w.height.saturating_sub(TASKBAR_H + 20) as i32;

        for (i, t) in self.anim.toasts.iter().enumerate() {
            let alpha = t.alpha();
            if alpha == 0 { continue; }
            let x = (base_x + t.offset_x()) as usize;
            let y = (base_y - ((i as i32 + 1) * (toast_h + gap) as i32)) as usize;

            let accent = match t.kind {
                anim::ToastKind::Info    => p.accent,
                anim::ToastKind::Success => Color { r: 60, g: 180, b: 90 },
                anim::ToastKind::Warn    => p.danger,
            };

            if alpha == 255 {
                w.fill_round_rect_aa(x, y, toast_w, toast_h, 8, p.window_bg_alt);
                w.fill_round_rect_aa(x, y, 4, toast_h, 2, accent);
                w.draw_text_at(x + 14, y + (toast_h - FONT_HEIGHT) / 2, &t.text, p.text, p.window_bg_alt);
            } else {
                w.fill_round_rect_aa(x, y, toast_w, toast_h, 8, p.window_bg_alt);
                w.fill_round_rect_aa(x, y, 4, toast_h, 2, accent);
                w.draw_text_at(x + 14, y + (toast_h - FONT_HEIGHT) / 2, &t.text, p.text, p.window_bg_alt);
                let wp_color = p.wallpaper_top;
                let inv = 255 - alpha;
                if inv > 0 {
                    for yy in y..(y + toast_h) {
                        for xx in x..(x + toast_w) {
                            w.blend_pixel(xx, yy, wp_color, inv);
                        }
                    }
                }
            }
        }
    }

    fn draw_ctx_menu(&self, w: &mut Writer, x: i32, y: i32, row: usize) {
        let p = theme::palette();
        let is_item = row != usize::MAX;
        let items: &[&str] = if is_item {
            &["Открыть", "Копировать", "Удалить"]
        } else {
            &["Вставить", "Новая папка", "Новый файл"]
        };

        let menu_h = items.len() * CTX_ITEM_H + 8;
        let x = x.max(0) as usize;
        let y = y.max(0) as usize;

        w.fill_round_rect(x + 3, y + 3, CTX_MENU_W, menu_h, 8, p.shadow);
        w.fill_round_rect(x, y, CTX_MENU_W, menu_h, 8, p.window_bg_alt);

        for (i, item) in items.iter().enumerate() {
            let iy = y + 4 + i * CTX_ITEM_H;
            let hover = widgets::hit(self.hover.0, self.hover.1, x, iy, CTX_MENU_W, CTX_ITEM_H);
            let bg = if hover { p.accent } else { p.window_bg_alt };
            if hover {
                w.fill_round_rect(x + 4, iy, CTX_MENU_W - 8, CTX_ITEM_H, 5, bg);
            }
            w.draw_text_at(x + 14, iy + 5, item, p.text, bg);
        }
    }

    fn draw_window(&self, w: &mut Writer, win: &Window, active: bool) {
        let p = theme::palette();
        let x = win.x.max(0) as usize;
        let y = win.y.max(0) as usize;
        let ww = win.w;
        let wh = win.h;

        w.fill_round_rect_aa(x + 4, y + 4, ww, wh, 10, p.shadow);
        w.fill_round_rect_aa(x, y, ww, wh, 10, p.window_bg);

        let title_bg = if active { p.title_active } else { p.title_inactive };
        let title_alpha = if active { p.title_alpha } else { 255 };

        if title_alpha < 255 {
            w.fill_round_rect_aa_blend(x, y, ww, TITLE_H + 10, 10, title_bg, title_alpha);
            w.fill_rect(x, y + TITLE_H, ww, 10, p.window_bg);
        } else {
            w.fill_round_rect_aa(x, y, ww, TITLE_H + 10, 10, title_bg);
            w.fill_rect(x, y + TITLE_H, ww, 10, p.window_bg);
        }
        w.draw_text_at(x + 14, y + (TITLE_H - FONT_HEIGHT) / 2, &win.title, p.text, title_bg);

        let cb_x = x + ww.saturating_sub(12 + CLOSE_BTN_W);
        let cb_y = y + (TITLE_H - 20) / 2;
        self.draw_close_btn(w, cb_x, cb_y);
        let mb_x = cb_x.saturating_sub(6 + MIN_BTN_W);
        self.draw_min_btn(w, mb_x, cb_y);

        match &win.content {
            App::Notepad {
                text, file, modified, mode, cursor, selection_anchor,
            } => self.draw_notepad(
                w, win, active, text, *cursor, *selection_anchor,
                file.as_deref(), *modified, mode,
            ),
            App::Explorer { path, selected, mode, scroll, .. } => {
                self.draw_explorer(w, win, path, selected, mode, *scroll)
            }
            App::Todo { items, checked, selected, input } => {
                self.draw_todo(w, win, items, checked, *selected, input)
            }
            App::Calculator { display, .. } => self.draw_calc(w, win, display),
            App::Paint { canvas, w: cw, h: chh, .. } => {
                self.draw_paint(w, win, canvas, *cw, *chh)
            }
        }
    }

    fn draw_close_btn(&self, w: &mut Writer, x: usize, y: usize) {
        let p = theme::palette();
        let hover = widgets::hit(self.hover.0, self.hover.1, x, y, CLOSE_BTN_W, 20);
        let bg = if hover { p.danger } else { p.window_bg_alt };
        w.fill_round_rect(x, y, CLOSE_BTN_W, 20, 6, bg);
        let cx = x + CLOSE_BTN_W / 2;
        let cy = y + 10;
        w.fill_rect(cx - 3, cy - 1, 7, 2, p.text);
        w.fill_rect(cx - 1, cy - 3, 2, 7, p.text);
    }

    fn draw_min_btn(&self, w: &mut Writer, x: usize, y: usize) {
        let p = theme::palette();
        let hover = widgets::hit(self.hover.0, self.hover.1, x, y, MIN_BTN_W, 20);
        let bg = if hover { p.border } else { p.window_bg_alt };
        w.fill_round_rect(x, y, MIN_BTN_W, 20, 6, bg);
        let cx = x + MIN_BTN_W / 2;
        let cy = y + 12;
        w.fill_rect(cx - 4, cy, 8, 2, p.text);
    }

    fn draw_notepad(
        &self, w: &mut Writer, win: &Window, active: bool, text: &str,
        cursor: usize, selection_anchor: Option<usize>,
        file: Option<&str>, modified: bool, mode: &NotepadMode,
    ) {
        let p = theme::palette();
        let x = win.x.max(0) as usize;
        let y = win.y.max(0) as usize;
        let pad = 14;
        let field_x = x + pad;
        let field_y = y + TITLE_H + 6;
        let field_w = win.w.saturating_sub(pad * 2);
        let field_h = win.h.saturating_sub(TITLE_H + pad + 8);
        w.fill_round_rect(field_x, field_y, field_w, field_h, 6, p.field_bg);

        let chars: Vec<char> = text.chars().collect();
        let cursor = cursor.min(chars.len());
        let sel = super::notepad::selection_range(cursor, selection_anchor);

        let px = field_x + 8;
        let py = field_y + 8;
        let max_chars = field_w.saturating_sub(16) / 9;

        let mut line = 0usize;
        let mut col = 0usize;
        let mut cursor_pixel: Option<(usize, usize)> = None;

        for (i, c) in chars.iter().enumerate() {
            if i == cursor {
                cursor_pixel = Some((px + col * 9, py + line * FONT_HEIGHT));
            }
            if *c == '\n' {
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
            let in_sel = matches!(sel, Some((lo, hi)) if i >= lo && i < hi);
            let bg = if in_sel { p.accent } else { p.field_bg };
            let fg = if in_sel { Color::WHITE } else { p.text };
            w.draw_text_at(px + col * 9, py + line * FONT_HEIGHT, &c.to_string(), fg, bg);
            col += 1;
        }

        if cursor >= chars.len() {
            cursor_pixel = Some((px + col * 9, py + line * FONT_HEIGHT));
        }

        if active {
            if let Some((cx, cy)) = cursor_pixel {
                if cx + 2 <= field_x + field_w && cy + FONT_HEIGHT <= field_y + field_h {
                    w.fill_rect(cx, cy, 2, FONT_HEIGHT, p.accent);
                }
            }
        }

        let status = match (file, modified) {
            (Some(f), true) => format!("{} *    Ctrl+S сохранить", f),
            (Some(f), false) => f.to_string(),
            (None, _) => "(без имени)  Ctrl+S сохранить".to_string(),
        };
        w.draw_text_at(field_x + 4, y + win.h.saturating_sub(14), &status, p.text_muted, p.window_bg);

        let dlg = match mode {
            NotepadMode::SaveAs { name } => Some(("Сохранить как (8.3):", name.as_str())),
            NotepadMode::Open { name } => Some(("Открыть файл (8.3):", name.as_str())),
            NotepadMode::Browse => None,
        };
        if let Some((label, value)) = dlg {
            let dlg_w = 360usize;
            let dlg_h = 120usize;
            let dlg_x = x + (win.w.saturating_sub(dlg_w)) / 2;
            let dlg_y = y + (win.h.saturating_sub(dlg_h)) / 2;

            w.fill_round_rect(dlg_x + 3, dlg_y + 3, dlg_w, dlg_h, 10, p.shadow);
            w.fill_round_rect(dlg_x, dlg_y, dlg_w, dlg_h, 10, p.window_bg_alt);
            w.draw_text_at(dlg_x + 16, dlg_y + 16, label, p.text, p.window_bg_alt);

            widgets::text_field_modern(
                w, dlg_x + 16, dlg_y + 46, dlg_w - 32, widgets::FIELD_H, value, true,
            );
            w.draw_text_at(dlg_x + 16, dlg_y + dlg_h - 24, "Enter - OK, Esc - Отмена", p.text_muted, p.window_bg_alt);
        }
    }

    fn draw_explorer(
        &self, w: &mut Writer, win: &Window, path: &str,
        selected: &[usize], mode: &ExplorerMode, scroll: usize,
    ) {
        let p = theme::palette();
        let x = win.x.max(0) as usize;
        let y = win.y.max(0) as usize;
        let ww = win.w;
        let wh = win.h;
        let on_disk = path.starts_with("C:");

        let toolbar_y = y + TITLE_H + 8;
        let back_x = x + 10;
        let up_x = back_x + 48;
        let nf_x = up_x + 48;
        let fl_x = nf_x + 90;
        let del_x = fl_x + 90;
        let ren_x = del_x + 80;

        widgets::button_modern(w, back_x, toolbar_y, 42, EXP_TOOLBAR_H, "", p.window_bg_alt, p.text, false);
        draw_arrow_left(w, back_x, toolbar_y + 10, p.text);
        widgets::button_modern(w, up_x, toolbar_y, 42, EXP_TOOLBAR_H, "", p.window_bg_alt, p.text, false);
        draw_arrow_up(w, up_x, toolbar_y + 10, p.text);

        let nf_label = if on_disk { "Файл" } else { "Папка" };
        widgets::button_modern(w, nf_x, toolbar_y, 86, EXP_TOOLBAR_H, nf_label, p.window_bg_alt, p.text, false);
        draw_plus_icon(w, nf_x + 6, toolbar_y + 10, p.text);
        widgets::button_modern(w, fl_x, toolbar_y, 86, EXP_TOOLBAR_H, "Файл", p.window_bg_alt, p.text, false);
        draw_plus_icon(w, fl_x + 6, toolbar_y + 10, p.text);
        widgets::button_modern(w, del_x, toolbar_y, 76, EXP_TOOLBAR_H, "", p.window_bg_alt, p.text, false);
        draw_trash_icon(w, del_x + 4, toolbar_y + 10, p.text);
        widgets::button_modern(w, ren_x, toolbar_y, 76, EXP_TOOLBAR_H, "", p.window_bg_alt, p.text, false);
        draw_pencil_icon(w, ren_x + 4, toolbar_y + 2, p.text);

        let addr_x = ren_x + 86;
        let addr_w = (x + ww).saturating_sub(addr_x + 12);
        w.fill_round_rect(addr_x, toolbar_y + 4, addr_w, EXP_TOOLBAR_H - 8, 5, p.window_bg_alt);
        w.draw_text_at(
            addr_x + 10,
            toolbar_y + 4 + (EXP_TOOLBAR_H - 8 - FONT_HEIGHT) / 2,
            path, p.text, p.window_bg_alt,
        );

        let body_y = toolbar_y + EXP_TOOLBAR_H + 8;
        let body_h = (y + wh).saturating_sub(body_y + 12);

        let sidebar_x = x + 10;
        w.fill_round_rect(sidebar_x, body_y, EXP_SIDEBAR_W, body_h, 6, p.window_bg_alt);
        let items: [&str; 6] = ["Домой", "Рабочий стол", "Документы", "Загрузки", "Система", "Диск (C:)"];
        for (i, name) in items.iter().enumerate() {
            let ry = body_y + 14 + i * 30;
            let active_here = (name == &"Диск (C:)" && on_disk) || (name == &"Домой" && path == "/");
            let fg = if active_here { p.accent } else { p.text };
            w.draw_text_at(sidebar_x + 12, ry + 5, name, fg, p.window_bg_alt);
        }

        let view_x = x + EXP_SIDEBAR_W + 20;
        let view_y = body_y;
        let view_w = ww.saturating_sub(EXP_SIDEBAR_W + 30);
        let view_h = body_h;
        let view_bg = p.list_bg;
        w.fill_round_rect(view_x, view_y, view_w, view_h, 6, view_bg);

        w.draw_text_at(view_x + 14, view_y + 10, "Имя", p.text_muted, view_bg);
        w.draw_text_at(view_x + view_w.saturating_sub(120), view_y + 10, "Размер", p.text_muted, view_bg);

        let entries: Vec<(String, bool, u32)> = if on_disk {
            crate::vfs::fat32_list_dir(path)
        } else {
            crate::vfs::ramfs_list_meta(path)
                .into_iter()
                .map(|(n, d, s)| (n, d, s as u32))
                .collect()
        };

        let list_y_start = view_y + 34;
        let max_rows = (view_y + view_h).saturating_sub(list_y_start) / EXP_ROW_H;
        let total = entries.len();
        let max_scroll = total.saturating_sub(max_rows);
        let scroll = scroll.min(max_scroll);

        for (i, (name, is_dir, size)) in entries.iter().enumerate().skip(scroll).take(max_rows) {
            let ry = list_y_start + (i - scroll) * EXP_ROW_H;
            let sel = selected.contains(&i);
            let bg = if sel { p.accent } else { view_bg };
            if sel {
                w.fill_round_rect(view_x + 6, ry, view_w.saturating_sub(12), EXP_ROW_H - 2, 5, bg);
            }
            draw_file_icon(w, view_x + 12, ry + 4, *is_dir, bg);
            let fg = if sel { Color::WHITE } else { p.text };
            w.draw_text_at(view_x + 48, ry + 5, name, fg, bg);
            if !*is_dir {
                let sz = format!("{} Б", size);
                let sz_fg = if sel { Color::WHITE } else { p.text_muted };
                w.draw_text_at(view_x + view_w.saturating_sub(120), ry + 5, &sz, sz_fg, bg);
            }
        }

        // Новый scrollbar_v из widgets.
        if max_scroll > 0 {
            let sb_x = view_x + view_w - SCROLLBAR_W - 2;
            let sb_y = view_y + 34;
            let sb_h = view_h.saturating_sub(34);
            widgets::scrollbar_v(w, sb_x, sb_y, SCROLLBAR_W, sb_h,
                scroll, max_scroll, total, max_rows);
        }

        let dlg: Option<(String, &str, bool)> = match mode {
            ExplorerMode::NewFolder { name } => Some(("Имя новой папки:".to_string(), name.as_str(), false)),
            ExplorerMode::NewFile { name } => Some(("Имя нового файла (8.3):".to_string(), name.as_str(), false)),
            ExplorerMode::Rename { name } => Some(("Новое имя (8.3):".to_string(), name.as_str(), false)),
            ExplorerMode::ConfirmDelete { count, .. } => {
                let text = if *count == 1 { "Удалить этот элемент?".to_string() } else { format!("Удалить {} элементов?", count) };
                Some((text, "", true))
            }
            ExplorerMode::Browse => None,
        };
        if let Some((label, value, is_confirm)) = dlg {
            let dlg_w = 360usize;
            let dlg_h = 120usize;
            let dlg_x = x + (ww.saturating_sub(dlg_w)) / 2;
            let dlg_y = y + (wh.saturating_sub(dlg_h)) / 2;

            w.fill_round_rect(dlg_x + 3, dlg_y + 3, dlg_w, dlg_h, 10, p.shadow);
            w.fill_round_rect(dlg_x, dlg_y, dlg_w, dlg_h, 10, p.window_bg_alt);

            if is_confirm {
                w.draw_text_at(dlg_x + 16, dlg_y + 30, &label, p.text, p.window_bg_alt);
                w.draw_text_at(dlg_x + 16, dlg_y + 70, "Enter - Да, Esc - Отмена", p.text_muted, p.window_bg_alt);
                w.draw_border(dlg_x, dlg_y, dlg_w, dlg_h, 2, p.danger);
            } else {
                w.draw_text_at(dlg_x + 16, dlg_y + 16, &label, p.text, p.window_bg_alt);
                widgets::text_field_modern(w, dlg_x + 16, dlg_y + 46, dlg_w - 32, widgets::FIELD_H, value, true);
                w.draw_text_at(dlg_x + 16, dlg_y + dlg_h - 24, "Enter - OK, Esc - Отмена", p.text_muted, p.window_bg_alt);
            }
        }

        if !self.clipboard.is_empty() {
            let hint = format!("В буфере: {} ({} элементов)", self.clipboard_from, self.clipboard.len());
            w.draw_text_at(view_x + 8, view_y + view_h - 14, &hint, p.text_muted, view_bg);
        }
    }

    fn draw_todo(
        &self,
        w: &mut Writer,
        win: &Window,
        items: &[String],
        checked: &[bool],
        selected: Option<usize>,
        input: &str,
    ) {
        let p = theme::palette();
        let x = win.x.max(0) as usize;
        let y = win.y.max(0) as usize;
        let pad = 16;
        let field_x = x + pad;
        let field_y = y + TITLE_H + pad;
        let field_w = win.w.saturating_sub(pad * 3 + 80);
        let add_btn_x = field_x + field_w + 8;

        widgets::text_field_modern(w, field_x, field_y, field_w, widgets::FIELD_H, input, true);
        widgets::button_modern(w, add_btn_x, field_y, 72, widgets::FIELD_H, "Добавить", p.accent, Color::WHITE, false);

        let list_x = x + pad;
        let list_y = field_y + widgets::FIELD_H + 12;
        let list_w = win.w.saturating_sub(pad * 2);
        let list_h = win.h.saturating_sub(TITLE_H + widgets::FIELD_H + 4 * pad + widgets::BUTTON_H);
        let list_bg = p.field_bg;
        w.fill_round_rect(list_x, list_y, list_w, list_h, 4, list_bg);

        let row_h = 28usize;
        let max_rows = (list_h.saturating_sub(12)) / row_h;
        for (i, item) in items.iter().take(max_rows).enumerate() {
            let iy = list_y + 6 + i * row_h;
            let is_sel = selected == Some(i);
            let is_chk = checked.get(i).copied().unwrap_or(false);
            if is_sel {
                w.fill_round_rect(list_x + 4, iy, list_w - 8, row_h - 2, 4, p.accent);
            }
            // Checkbox 18x18 слева.
            let cb_x = list_x + 12;
            let cb_y = iy + (row_h - 18) / 2;
            let cb_size = 18usize;
            let box_bg = if is_sel { p.accent } else { p.window_bg_alt };
            w.fill_round_rect(cb_x, cb_y, cb_size, cb_size, 4, box_bg);
            for k in 0..cb_size {
                w.set_pixel(cb_x, cb_y + k, p.border);
                w.set_pixel(cb_x + cb_size - 1, cb_y + k, p.border);
                w.set_pixel(cb_x + k, cb_y, p.border);
                w.set_pixel(cb_x + k, cb_y + cb_size - 1, p.border);
            }
            if is_chk {
                let cx = cb_x + cb_size / 2;
                let cy = cb_y + cb_size / 2;
                for k in 0..5 {
                    w.set_pixel(cx - 5 + k, cy + k - 1, p.accent);
                    w.set_pixel(cx - 5 + k, cy + k, p.accent);
                }
                for k in 0..8 {
                    w.set_pixel(cx - 1 + k, cy - 1 - k + 4, p.accent);
                    w.set_pixel(cx - 1 + k, cy - k + 4, p.accent);
                }
            }
            // Текст. Если отмечено — серым.
            let (fg, bg) = if is_sel {
                (Color::WHITE, p.accent)
            } else if is_chk {
                (p.text_muted, list_bg)
            } else {
                (p.text, list_bg)
            };
            w.draw_text_at(cb_x + cb_size + 10, iy + (row_h - FONT_HEIGHT) / 2, item, fg, bg);
        }

        let remove_btn_y = list_y + list_h + 10;
        widgets::button_modern(w, list_x, remove_btn_y, 120, widgets::BUTTON_H, "Удалить", p.danger, Color::WHITE, false);
    }

    fn draw_calc(&self, w: &mut Writer, win: &Window, display: &str) {
        let p = theme::palette();
        let layout = calc_layout(win);
        let (dx, dy, dw, dh) = (layout.display_x, layout.display_y, layout.display_w, layout.display_h);
        let (bx, by, step) = (layout.grid_x, layout.grid_y, layout.step);

        w.fill_round_rect(dx, dy, dw, dh, 6, p.field_bg);
        let tw = Writer::text_width(display);
        w.draw_text_at(dx + dw.saturating_sub(tw + 12), dy + (dh - FONT_HEIGHT) / 2, display, p.text, p.field_bg);

        widgets::button_modern(w, bx, by.saturating_sub(50), 64, 40, "C", p.danger, Color::WHITE, false);

        let labels = [["7","8","9","/"],["4","5","6","*"],["1","2","3","-"],["0",".","=","+"]];
        let bw = 64usize;
        let bh = 44usize;
        for (r, row) in labels.iter().enumerate() {
            for (c, label) in row.iter().enumerate() {
                let is_op = matches!(*label, "/" | "*" | "-" | "+" | "=");
                let bg = if is_op { p.accent } else { p.window_bg_alt };
                widgets::button_modern(w, bx + c * step, by + r * (bh + 6), bw, bh, label, bg, Color::WHITE, false);
            }
        }
    }

    fn draw_paint(&self, w: &mut Writer, win: &Window, canvas: &[u8], cw: usize, ch: usize) {
        let p = theme::palette();
        let x = win.x.max(0) as usize;
        let y = win.y.max(0) as usize;
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
        w.draw_border(cx, cy, cw, ch, 1, p.text_muted);
        widgets::button_modern(w, cx, cy + ch + 10, 100, 30, "Очистить", p.window_bg_alt, p.text, false);
    }

    fn draw_taskbar(&self, w: &mut Writer) {
        let p = theme::palette();
        let h = w.height;
        let y = h.saturating_sub(TASKBAR_H);

        if p.blur_radius > 0 {
            w.blur_rect(0, y, w.width, TASKBAR_H, p.blur_radius);
        }

        w.fill_rect_blend(0, y, w.width, TASKBAR_H, p.taskbar_bg, p.taskbar_alpha);
        w.fill_rect(0, y, w.width, 1, p.border);

        let start_hover = widgets::hit(self.hover.0, self.hover.1, 6, y + 6, 70, TASKBAR_H.saturating_sub(12));
        let start_bg = if self.start_pressed || start_hover { p.accent } else { p.window_bg_alt };
        widgets::button_modern(w, 6, y + 6, 70, TASKBAR_H.saturating_sub(12), "Пуск", start_bg, Color::WHITE, false);

        let mut bx = 86;
        for (idx, win) in self.windows.iter().enumerate() {
            let label = &win.title;
            let bw = Writer::text_width(label) + 24;
            let pressed = idx == self.active && !win.minimized;
            let bg = if pressed { p.accent } else { p.window_bg_alt };
            widgets::button_modern(w, bx, y + 6, bw, TASKBAR_H.saturating_sub(12), label, bg, Color::WHITE, false);
            bx += bw + 4;
        }

        let secs = uptime_secs();
        let text = format!("{:02}:{:02}", (secs / 60) % 60, secs % 60);
        let tw = Writer::text_width(&text);
        w.draw_text_at(w.width.saturating_sub(tw + 16), y + (TASKBAR_H - FONT_HEIGHT) / 2, &text, p.text, p.taskbar_bg);
    }

    fn draw_start_menu(&self, w: &mut Writer) {
        let p = theme::palette();
        let h = w.height;
        let menu_h = START_MENU_H;
        let menu_w = 240;
        let menu_x = 6;
        let menu_y = h.saturating_sub(TASKBAR_H + menu_h + 6);

        if p.blur_radius > 0 {
            w.blur_rect(menu_x, menu_y, menu_w, menu_h, p.blur_radius);
        }

        w.fill_round_rect_aa(menu_x + 4, menu_y + 4, menu_w, menu_h, 12, p.shadow);
        w.fill_round_rect_aa_blend(menu_x, menu_y, menu_w, menu_h, 12, p.window_bg_alt, p.menu_alpha);

        w.draw_text_at(menu_x + 16, menu_y + 14, "Rust OS", p.text, p.window_bg_alt);
        w.draw_text_at(menu_x + 16, menu_y + 14 + FONT_HEIGHT, "v0.9", p.text_muted, p.window_bg_alt);

        for (i, item) in START_MENU_ITEMS.iter().enumerate() {
            let iy = menu_y + 60 + i * 32;
            let hover = widgets::hit(self.hover.0, self.hover.1, menu_x + 8, iy, menu_w.saturating_sub(16), 28);
            let bg = if hover { p.accent } else { p.window_bg_alt };
            if hover {
                w.fill_round_rect(menu_x + 8, iy, menu_w.saturating_sub(16), 28, 6, bg);
            }
            w.draw_text_at(menu_x + 20, iy + 5, item, p.text, bg);
        }
    }
}

fn draw_foreign(w: &mut Writer, fw: &crate::win::ForeignWindow) {
    let p = theme::palette();
    let x = fw.x.max(0) as usize;
    let y = fw.y.max(0) as usize;
    let ww = fw.w as usize;
    let hh = fw.h as usize;
    let title_h = crate::win::TITLE_BAR_H as usize;

    w.fill_round_rect(x + 4, y + 4, ww, hh + title_h, 10, p.shadow);
    w.fill_round_rect(x, y, ww, hh + title_h, 10, p.window_bg);

    let title_bg = p.title_active;
    w.fill_round_rect(x, y, ww, title_h, 10, title_bg);
    w.draw_text_at(x + 10, y + 5, &fw.title, p.text, title_bg);

    let cx = x;
    let cy = y + title_h;
    for yy in 0..hh {
        for xx in 0..ww {
            let off = (yy * ww + xx) * 4;
            if off + 2 >= fw.shadow.len() { continue; }
            let b = fw.shadow[off];
            let g = fw.shadow[off + 1];
            let r = fw.shadow[off + 2];
            w.set_pixel(cx + xx, cy + yy, Color { r, g, b });
        }
    }
}

fn draw_wallpaper(w: &mut Writer, wp: &super::WallpaperBuf) {
    if wp.width == 0 || wp.height == 0 { return; }
    let scr_w = w.width;
    let scr_h = w.height;

    if wp.width == scr_w && wp.height == scr_h {
        for y in 0..scr_h {
            for x in 0..scr_w {
                let off = (y * wp.width + x) * 3;
                let color = Color {
                    r: wp.pixels[off],
                    g: wp.pixels[off + 1],
                    b: wp.pixels[off + 2],
                };
                w.set_pixel(x, y, color);
            }
        }
        return;
    }

    let scale_x = ((wp.width as u64) << 16) / scr_w as u64;
    let scale_y = ((wp.height as u64) << 16) / scr_h as u64;

    for y in 0..scr_h {
        let src_y = ((y as u64 * scale_y) >> 16) as usize;
        let src_y = src_y.min(wp.height - 1);
        for x in 0..scr_w {
            let src_x = ((x as u64 * scale_x) >> 16) as usize;
            let src_x = src_x.min(wp.width - 1);
            let off = (src_y * wp.width + src_x) * 3;
            let color = Color {
                r: wp.pixels[off],
                g: wp.pixels[off + 1],
                b: wp.pixels[off + 2],
            };
            w.set_pixel(x, y, color);
        }
    }
}