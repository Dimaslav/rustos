//! Единый расчёт layout'а таскбара — используется и в draw.rs, и в input.rs,
//! чтобы зоны кликов совпадали с визуальными кнопками.
//!
//! Раньше ширина/шаг кнопки считались по-разному в двух местах и расходились,
//! из-за чего клики по таскбару «промахивались».

use alloc::string::String;
use alloc::vec::Vec;

use crate::framebuffer::Writer;

pub const START_BTN_X: usize = 6;
pub const START_BTN_W: usize = 70;
pub const SEP_X: usize = 82;
pub const FIRST_TASK_X: usize = 96;
pub const TASK_GAP: usize = 10;

/// Геометрия одной кнопки задачи.
#[derive(Clone, Copy, Debug)]
pub struct TaskButton {
    pub x: usize,
    pub w: usize,
}

/// Список кнопок задач в таскбаре для набора заголовков окон.
/// Обрезает по правой границе фреймбуфера.
pub fn task_buttons(titles: &[String], fb_w: usize) -> Vec<TaskButton> {
    let mut out = Vec::with_capacity(titles.len());
    let mut bx = FIRST_TASK_X;
    for title in titles {
        let label_w = Writer::text_width(title);
        // Симметричный padding: 14 слева + 16 под иконку + 8 отступ + текст + 14 справа
        let bw = 14 + 16 + 8 + label_w + 14;
        if bx + bw > fb_w {
            break;
        }
        out.push(TaskButton { x: bx, w: bw });
        bx += bw + TASK_GAP;
    }
    out
}

/// Высота кнопки на таскбаре.
pub fn button_h(taskbar_h: usize) -> usize {
    taskbar_h.saturating_sub(14)
}

/// Y-позиция кнопки на таскбаре (относительно верха фреймбуфера).
pub fn button_y(taskbar_y: usize, taskbar_h: usize) -> usize {
    taskbar_y + (taskbar_h - button_h(taskbar_h)) / 2
}