//! Типы состояния: приложения, окна, режимы редактирования, константы.

use alloc::string::String;
use alloc::vec::Vec;

pub const TITLE_H: usize = 30;
pub const TASKBAR_H: usize = 42;
pub const CLOSE_BTN_W: usize = 26;
pub const MIN_BTN_W: usize = 26;

pub const ICON_X: usize = 32;
pub const ICON_Y: usize = 32;
pub const ICON_STEP: usize = 110;

pub const EXP_TOOLBAR_H: usize = 38;
pub const EXP_ROW_H: usize = 28;
pub const EXP_SIDEBAR_W: usize = 160;

pub const CTX_MENU_W: usize = 160;
pub const CTX_ITEM_H: usize = 26;

pub const SCROLLBAR_W: usize = 10;

pub const START_MENU_ITEMS: [&str; 6] = [
    "Программы",
    "Документы",
    "Сменить тему",
    "О системе",
    "Перезагрузка",
    "Выключение",
];

pub const START_MENU_H: usize = 272;

/// Зона автоматической подгонки окна при перетаскивании к краю экрана.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SnapZone {
    Left,
    Right,
    Top,
}

#[derive(Clone, Copy)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    DocStart,
    DocEnd,
    WordLeft,
    WordRight,
    PageUp,
    PageDown,
}

#[derive(Clone)]
pub enum ExplorerMode {
    Browse,
    NewFolder { name: String },
    NewFile { name: String },
    Rename { name: String },
    ConfirmDelete { count: usize, on_disk: bool },
}

#[derive(Clone)]
pub enum NotepadMode {
    Browse,
    SaveAs { name: String },
    Open { name: String },
}

#[derive(Clone)]
pub struct ClipboardItem {
    pub name: String,
    pub data: Vec<u8>,
    pub is_dir: bool,
}

#[derive(Clone)]
pub enum App {
    Notepad {
        text: String,
        file: Option<String>,
        modified: bool,
        mode: NotepadMode,
        /// Индекс символа (не байта), где стоит курсор.
        cursor: usize,
        /// Если `Some`, есть активное выделение от этого индекса до `cursor`.
        selection_anchor: Option<usize>,
    },
    Explorer {
        path: String,
        selected: Vec<usize>,
        anchor: Option<usize>,
        mode: ExplorerMode,
        history: Vec<String>,
        last_click: Option<(u64, usize)>,
        ctx_menu: Option<(i32, i32, usize)>,
        scroll: usize,
        scroll_drag: bool,
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
pub enum AppKind {
    Explorer,
    Notepad,
    Todo,
    Calculator,
    Paint,
}

pub struct Window {
    pub(in crate::gui) x: i32,
    pub(in crate::gui) y: i32,
    pub(in crate::gui) w: usize,
    pub(in crate::gui) h: usize,
    pub(in crate::gui) title: String,
    pub(in crate::gui) content: App,
    pub(in crate::gui) minimized: bool,
    pub(in crate::gui) restore_rect: Option<(i32, i32, usize, usize)>,
}

pub struct Drag {
    pub(in crate::gui) idx: usize,
    pub(in crate::gui) ox: i32,
    pub(in crate::gui) oy: i32,
}

pub const CURSOR: &[(i32, i32, u8)] = &[
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

pub const CURSOR_W: i32 = 12;
pub const CURSOR_H: i32 = 19;