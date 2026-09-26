//! Общие helpers для user-программ.
#![no_std]

use core::arch::asm;

#[inline(always)]
pub unsafe fn syscall(nr: u64, a1: u64, a2: u64, a3: u64) -> u64 {
    let ret: u64;
    asm!(
        "int 0x80",
        inlateout("rax") nr => ret,
        in("rdi") a1,
        in("rsi") a2,
        in("rdx") a3,
        clobber_abi("sysv64"),
        options(nostack)
    );
    ret
}

#[inline(always)]
pub fn write(s: &[u8]) {
    unsafe { syscall(1, 1, s.as_ptr() as u64, s.len() as u64); }
}

#[inline(always)]
pub fn sleep_ms(ms: u64) {
    unsafe { syscall(2, ms, 0, 0); }
}

#[inline(always)]
pub fn exit(code: u64) -> ! {
    unsafe { syscall(0, code, 0, 0); }
    loop {}
}

pub const WIN_BUF_VADDR: u64 = 0x0100_0000;

#[inline(always)]
pub fn create_window(w: u64, h: u64, title: &[u8]) -> u64 {
    unsafe { syscall(20, w, h, title.as_ptr() as u64) }
}

#[inline(always)]
pub fn destroy_window(id: u64) -> u64 {
    unsafe { syscall(21, id, 0, 0) }
}

#[inline(always)]
pub fn present(id: u64) -> u64 {
    unsafe { syscall(22, id, 0, 0) }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct WinEvent {
    pub kind: u32,
    pub x: i32,
    pub y: i32,
    pub code: u32,
}

#[inline(always)]
pub fn poll_event(id: u64) -> Option<WinEvent> {
    let mut ev = WinEvent { kind: 0, x: 0, y: 0, code: 0 };
    let r = unsafe { syscall(23, id, &mut ev as *mut _ as u64, 0) };
    if r == 1 { Some(ev) } else { None }
}

#[inline(always)]
pub fn spawn_calc() -> u64 {
    unsafe { syscall(24, 0, 0, 0) }
}

#[inline(always)]
pub fn exec(name: &str) -> u64 {
    unsafe { syscall(9, name.as_ptr() as u64, name.len() as u64, 0) }
}

#[inline(always)]
pub fn read_key() -> u64 {
    unsafe { syscall(10, 0, 0, 0) }
}

#[inline(always)]
pub fn set_focus(v: bool) {
    unsafe { syscall(11, if v { 1 } else { 0 }, 0, 0); }
}

#[inline(always)]
pub fn put_pixel(buf: *mut u8, stride_px: usize, x: usize, y: usize, r: u8, g: u8, b: u8) {
    let off = (y * stride_px + x) * 4;
    unsafe {
        *buf.add(off) = b;
        *buf.add(off + 1) = g;
        *buf.add(off + 2) = r;
        *buf.add(off + 3) = 255;
    }
}