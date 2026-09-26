//! Система анимаций и toast-уведомлений.

use alloc::string::String;
use alloc::vec::Vec;
use crate::interrupts::ticks;

pub const OPEN_DURATION: u64 = 3;
pub const RECT_DURATION: u64 = 4;
pub const TOAST_DURATION: u64 = 54;

#[derive(Clone)]
pub struct WindowAnim {
    pub win_idx: usize,
    pub start: u64,
}

impl WindowAnim {
    pub fn new(win_idx: usize) -> Self {
        Self { win_idx, start: ticks() }
    }
    pub fn progress(&self) -> f32 {
        let elapsed = ticks().saturating_sub(self.start);
        if elapsed >= OPEN_DURATION { return 1.0; }
        elapsed as f32 / OPEN_DURATION as f32
    }
    pub fn done(&self) -> bool {
        ticks().saturating_sub(self.start) >= OPEN_DURATION
    }
    pub fn offset_y(&self) -> i32 {
        ((1.0 - self.progress()) * 24.0) as i32
    }
}

/// Что делать при завершении rect-анимации.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RectOnDone {
    /// Ничего.
    None,
    /// Установить `win.minimized = true`.
    Minimize,
}

/// Универсальная анимация прямоугольника окна:
/// snap, minimize, restore, maximize — все через неё.
#[derive(Clone)]
pub struct RectAnim {
    pub win_idx: usize,
    pub start: u64,
    pub from: (i32, i32, usize, usize),
    pub to: (i32, i32, usize, usize),
    pub on_done: RectOnDone,
}

impl RectAnim {
    pub fn new(
        win_idx: usize,
        from: (i32, i32, usize, usize),
        to: (i32, i32, usize, usize),
        on_done: RectOnDone,
    ) -> Self {
        Self { win_idx, start: ticks(), from, to, on_done }
    }

    pub fn progress(&self) -> f32 {
        let elapsed = ticks().saturating_sub(self.start);
        if elapsed >= RECT_DURATION { return 1.0; }
        // Ease-out quad.
        let t = elapsed as f32 / RECT_DURATION as f32;
        1.0 - (1.0 - t) * (1.0 - t)
    }

    pub fn done(&self) -> bool {
        ticks().saturating_sub(self.start) >= RECT_DURATION
    }

    pub fn current(&self) -> (i32, i32, usize, usize) {
        let p = self.progress();
        let li = |a: i32, b: i32| -> i32 { a + ((b - a) as f32 * p) as i32 };
        let lu = |a: usize, b: usize| -> usize {
            if b >= a { a + ((b - a) as f32 * p) as usize }
            else { a - ((a - b) as f32 * p) as usize }
        };
        (li(self.from.0, self.to.0), li(self.from.1, self.to.1),
         lu(self.from.2, self.to.2), lu(self.from.3, self.to.3))
    }
}

#[derive(Clone, Copy)]
pub enum ToastKind {
    Info,
    Success,
    Warn,
}

#[derive(Clone)]
pub struct Toast {
    pub text: String,
    pub kind: ToastKind,
    pub start: u64,
}

impl Toast {
    pub fn new(text: String, kind: ToastKind) -> Self {
        Self { text, kind, start: ticks() }
    }
    pub fn age(&self) -> u64 { ticks().saturating_sub(self.start) }
    pub fn done(&self) -> bool { self.age() >= TOAST_DURATION }

    pub fn alpha(&self) -> u8 {
        let age = self.age();
        if age < 4 {
            ((age * 255) / 4) as u8
        } else if age + 8 >= TOAST_DURATION {
            let rem = TOAST_DURATION.saturating_sub(age);
            ((rem * 255) / 8).min(255) as u8
        } else {
            255
        }
    }

    pub fn offset_x(&self) -> i32 {
        let age = self.age();
        if age >= 4 { 0 } else { ((4 - age) * 60) as i32 }
    }
}

pub struct AnimState {
    pub window_anims: Vec<WindowAnim>,
    pub rect_anim: Option<RectAnim>,
    pub toasts: Vec<Toast>,
}

impl AnimState {
    pub const fn new() -> Self {
        Self {
            window_anims: Vec::new(),
            rect_anim: None,
            toasts: Vec::new(),
        }
    }

    pub fn start_open(&mut self, win_idx: usize) {
        self.window_anims.retain(|a| a.win_idx != win_idx);
        self.window_anims.push(WindowAnim::new(win_idx));
    }

    pub fn start_rect(
        &mut self,
        win_idx: usize,
        from: (i32, i32, usize, usize),
        to: (i32, i32, usize, usize),
        on_done: RectOnDone,
    ) {
        self.rect_anim = Some(RectAnim::new(win_idx, from, to, on_done));
    }

    pub fn toast(&mut self, text: &str, kind: ToastKind) {
        self.toasts.push(Toast::new(text.into(), kind));
        while self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
    }

    /// Возвращает true, если что-то активно. RectAnim НЕ удаляется здесь —
    /// этим занимается `Wm` (нужно применить `on_done`).
    pub fn tick(&mut self) -> bool {
        let had = !self.window_anims.is_empty()
            || self.rect_anim.is_some()
            || !self.toasts.is_empty();
        self.window_anims.retain(|a| !a.done());
        self.toasts.retain(|t| !t.done());
        had
    }

    pub fn is_active(&self) -> bool {
        !self.window_anims.is_empty() || self.rect_anim.is_some() || !self.toasts.is_empty()
    }
}