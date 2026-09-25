use alloc::string::{String, ToString};
use alloc::vec::Vec;

use super::state::Direction;

pub const TAB_STR: &str = "    ";

pub fn chars_count(s: &str) -> usize {
    s.chars().count()
}

pub fn byte_of_char(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(s.len())
}

pub fn idx_to_lc(s: &str, idx: usize) -> (usize, usize) {
    let mut line = 0usize;
    let mut col = 0usize;
    for (i, c) in s.chars().enumerate() {
        if i >= idx {
            break;
        }
        if c == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    (line, col)
}

/// Индекс char'а в позиции (line, col). Если строка короче — возвращает
/// индекс её `\n` (или конца текста).
pub fn lc_to_idx(s: &str, line: usize, col: usize) -> usize {
    let mut cur_line = 0usize;
    let mut cur_col = 0usize;
    for (i, c) in s.chars().enumerate() {
        if cur_line == line && cur_col == col {
            return i;
        }
        if c == '\n' {
            if cur_line == line {
                return i;
            }
            cur_line += 1;
            cur_col = 0;
        } else {
            cur_col += 1;
        }
    }
    chars_count(s)
}

/// Границы строки, в которой находится `idx` (start включительно, end — `\n` или конец текста).
pub fn line_bounds(s: &str, idx: usize) -> (usize, usize) {
    let chars: Vec<char> = s.chars().collect();
    let idx = idx.min(chars.len());
    let mut start = idx;
    while start > 0 && chars[start - 1] != '\n' {
        start -= 1;
    }
    let mut end = idx;
    while end < chars.len() && chars[end] != '\n' {
        end += 1;
    }
    (start, end)
}

fn word_left(s: &str, idx: usize) -> usize {
    let chars: Vec<char> = s.chars().collect();
    let mut i = idx.min(chars.len());
    while i > 0 && chars[i - 1].is_whitespace() {
        i -= 1;
    }
    while i > 0 && !chars[i - 1].is_whitespace() {
        i -= 1;
    }
    i
}

fn word_right(s: &str, idx: usize) -> usize {
    let chars: Vec<char> = s.chars().collect();
    let mut i = idx.min(chars.len());
    while i < chars.len() && !chars[i].is_whitespace() {
        i += 1;
    }
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    i
}

/// Возвращает (lo, hi) если есть активное выделение (lo < hi).
pub fn selection_range(cursor: usize, anchor: Option<usize>) -> Option<(usize, usize)> {
    let a = anchor?;
    let (lo, hi) = if a <= cursor { (a, cursor) } else { (cursor, a) };
    if lo == hi {
        None
    } else {
        Some((lo, hi))
    }
}

pub fn selected_text(s: &str, cursor: usize, anchor: Option<usize>) -> Option<String> {
    let (lo, hi) = selection_range(cursor, anchor)?;
    let bl = byte_of_char(s, lo);
    let bh = byte_of_char(s, hi);
    Some(s[bl..bh].to_string())
}

pub fn delete_selection(text: &mut String, cursor: &mut usize, anchor: &mut Option<usize>) {
    if let Some((lo, hi)) = selection_range(*cursor, *anchor) {
        let bl = byte_of_char(text, lo);
        let bh = byte_of_char(text, hi);
        text.replace_range(bl..bh, "");
        *cursor = lo;
    }
    *anchor = None;
}

pub fn insert_char(
    text: &mut String,
    cursor: &mut usize,
    anchor: &mut Option<usize>,
    c: char,
) {
    delete_selection(text, cursor, anchor);
    let b = byte_of_char(text, *cursor);
    text.insert(b, c);
    *cursor += 1;
}

pub fn insert_str(
    text: &mut String,
    cursor: &mut usize,
    anchor: &mut Option<usize>,
    s: &str,
) {
    delete_selection(text, cursor, anchor);
    let b = byte_of_char(text, *cursor);
    text.insert_str(b, s);
    *cursor += s.chars().count();
}

pub fn delete_forward(text: &mut String, cursor: &mut usize, anchor: &mut Option<usize>) {
    if anchor.is_some() {
        delete_selection(text, cursor, anchor);
        return;
    }
    let total = chars_count(text);
    if *cursor >= total {
        return;
    }
    let b = byte_of_char(text, *cursor);
    text.remove(b);
}

pub fn delete_backward(text: &mut String, cursor: &mut usize, anchor: &mut Option<usize>) {
    if anchor.is_some() {
        delete_selection(text, cursor, anchor);
        return;
    }
    if *cursor == 0 {
        return;
    }
    *cursor -= 1;
    let b = byte_of_char(text, *cursor);
    text.remove(b);
}

pub fn select_all(text: &str, cursor: &mut usize, anchor: &mut Option<usize>) {
    *anchor = Some(0);
    *cursor = chars_count(text);
}

pub fn move_cursor(
    text: &str,
    cursor: &mut usize,
    anchor: &mut Option<usize>,
    dir: Direction,
    extend: bool,
) {
    let old = *cursor;

    // Если есть выделение и жмут стрелку без Shift — курсор сворачивается
    // к соответствующему краю выделения, выделение снимается.
    if !extend {
        if let Some((lo, hi)) = selection_range(*cursor, *anchor) {
            match dir {
                Direction::Left
                | Direction::WordLeft
                | Direction::Home
                | Direction::DocStart
                | Direction::Up
                | Direction::PageUp => {
                    *cursor = lo;
                }
                Direction::Right
                | Direction::WordRight
                | Direction::End
                | Direction::DocEnd
                | Direction::Down
                | Direction::PageDown => {
                    *cursor = hi;
                }
            }
            *anchor = None;
            return;
        }
    }

    let new_cursor = match dir {
        Direction::Left => old.saturating_sub(1),
        Direction::Right => (old + 1).min(chars_count(text)),
        Direction::Up => {
            let (line, col) = idx_to_lc(text, old);
            if line == 0 {
                old
            } else {
                lc_to_idx(text, line - 1, col)
            }
        }
        Direction::Down => {
            let (line, col) = idx_to_lc(text, old);
            let total_lines = text.chars().filter(|&c| c == '\n').count();
            if line >= total_lines {
                old
            } else {
                lc_to_idx(text, line + 1, col)
            }
        }
        Direction::Home => line_bounds(text, old).0,
        Direction::End => line_bounds(text, old).1,
        Direction::DocStart => 0,
        Direction::DocEnd => chars_count(text),
        Direction::WordLeft => word_left(text, old),
        Direction::WordRight => word_right(text, old),
        Direction::PageUp => {
            let (line, col) = idx_to_lc(text, old);
            let new_line = line.saturating_sub(10);
            lc_to_idx(text, new_line, col)
        }
        Direction::PageDown => {
            let (line, col) = idx_to_lc(text, old);
            let total_lines = text.chars().filter(|&c| c == '\n').count();
            let new_line = (line + 10).min(total_lines);
            lc_to_idx(text, new_line, col)
        }
    };

    if extend {
        if anchor.is_none() {
            *anchor = Some(old);
        }
        *cursor = new_cursor;
        if *anchor == Some(new_cursor) {
            *anchor = None;
        }
    } else {
        *cursor = new_cursor;
        *anchor = None;
    }
}

// ---------- Глобальный текстовый буфер обмена ----------

static TEXT_CLIPBOARD: spin::Mutex<String> = spin::Mutex::new(String::new());

pub fn clipboard_get() -> String {
    TEXT_CLIPBOARD.lock().clone()
}

pub fn clipboard_set(s: String) {
    *TEXT_CLIPBOARD.lock() = s;
}

pub fn clipboard_is_empty() -> bool {
    TEXT_CLIPBOARD.lock().is_empty()
}