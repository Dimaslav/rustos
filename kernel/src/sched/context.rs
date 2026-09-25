//! Ассемблерный context switch.
//!
//! Сохраняет callee-saved регистры (r15, r14, r13, r12, rbp, rbx) на текущий
//! стек, кладёт rsp в `*old_rsp`, загружает `new_rsp`, восстанавливает
//! регистры и делает `ret`.
//!
//! Соглашение SysV AMD64: caller-saved регистры (rax, rcx, rdx, rsi, rdi,
//! r8..r11) считаются «испорченными», callee-saved — сохранены.
//!
//! Первая «раскрутка» fresh-стека прыгает в entry-функцию; начальный rsp для
//! неё готовит `thread::init_stack`.
//!
//! Синтаксис — Intel (по умолчанию для global_asm! на x86_64).

use core::arch::global_asm;

global_asm!(
    ".globl switch_context",
    ".type switch_context, @function",
    "switch_context:",
    "    push r15",
    "    push r14",
    "    push r13",
    "    push r12",
    "    push rbp",
    "    push rbx",
    "    mov [rdi], rsp",
    "    mov rsp, rsi",
    "    pop rbx",
    "    pop rbp",
    "    pop r12",
    "    pop r13",
    "    pop r14",
    "    pop r15",
    "    ret",
    ".size switch_context, . - switch_context",
);

extern "C" {
    /// Переключиться с текущего контекста на другой.
    ///
    /// * `old_rsp` — куда сохранить rsp текущего потока;
    /// * `new_rsp` — какой rsp загрузить (сохранённый ранее или подготовленный
    ///   `thread::init_stack`).
    pub fn switch_context(old_rsp: *mut u64, new_rsp: u64);
}