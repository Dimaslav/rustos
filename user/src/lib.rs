//! Общие helpers для user-программ.
#![no_std]
#![feature(alloc_error_handler)]

extern crate alloc;

use core::arch::asm;
use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

// ---------- Syscall ABI ----------

#[inline(always)]
pub unsafe fn syscall(nr: u64, a1: u64, a2: u64, a3: u64) -> u64 {
    syscall4(nr, a1, a2, a3, 0)
}

#[inline(always)]
pub unsafe fn syscall4(nr: u64, a1: u64, a2: u64, a3: u64, a4: u64) -> u64 {
    let ret: u64;
    asm!(
        "int 0x80",
        inlateout("rax") nr => ret,
        in("rdi") a1,
        in("rsi") a2,
        in("rdx") a3,
        in("rcx") a4,
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
pub fn write_fd(fd: u64, s: &[u8]) -> u64 {
    unsafe { syscall(1, fd, s.as_ptr() as u64, s.len() as u64) }
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

#[inline(always)]
pub fn read_key() -> u64 {
    unsafe { syscall(10, 0, 0, 0) }
}

#[inline(always)]
pub fn set_focus(v: bool) {
    unsafe { syscall(11, if v { 1 } else { 0 }, 0, 0); }
}

#[inline(always)]
pub fn getpid() -> u64 { unsafe { syscall(4, 0, 0, 0) } }

#[inline(always)]
pub fn getppid() -> u64 { unsafe { syscall(36, 0, 0, 0) } }

#[inline(always)]
pub fn wait(pid: u64) -> Option<i32> {
    let r = unsafe { syscall(34, pid, 0, 0) };
    if r == u64::MAX { None } else { Some(r as i64 as i32) }
}

#[inline(always)]
pub fn kill(pid: u64) -> bool {
    unsafe { syscall(35, pid, 0, 0) == 0 }
}

#[inline(always)]
pub fn exec(name: &str) -> u64 {
    unsafe { syscall(9, name.as_ptr() as u64, name.len() as u64, 0) }
}

pub fn exec_argv(name: &str, args: &[&str]) -> u64 {
    let mut cstrings: alloc::vec::Vec<alloc::vec::Vec<u8>> = alloc::vec::Vec::new();
    for a in args {
        let mut v: alloc::vec::Vec<u8> = a.as_bytes().to_vec();
        v.push(0);
        cstrings.push(v);
    }
    let mut ptrs: alloc::vec::Vec<u64> = cstrings.iter().map(|v| v.as_ptr() as u64).collect();
    ptrs.push(0);
    unsafe {
        syscall4(
            37,
            name.as_ptr() as u64,
            name.len() as u64,
            ptrs.as_ptr() as u64,
            args.len() as u64,
        )
    }
}

// ---------- Файлы ----------

#[inline(always)]
pub fn open(path: &str) -> u64 {
    unsafe { syscall(5, path.as_ptr() as u64, path.len() as u64, 0) }
}

/// Открывает файл для записи. `append=false` — обнуляет файл, `true` — дописывает.
/// Возвращает fd или `u64::MAX`.
#[inline(always)]
pub fn open_write(path: &str, append: bool) -> u64 {
    unsafe {
        syscall(
            42,
            path.as_ptr() as u64,
            path.len() as u64,
            if append { 1 } else { 0 },
        )
    }
}

#[inline(always)]
pub fn read(fd: u64, buf: u64, len: u64) -> u64 {
    unsafe { syscall(6, fd, buf, len) }
}

#[inline(always)]
pub fn close(fd: u64) -> u64 {
    unsafe { syscall(7, fd, 0, 0) }
}

// ---------- Pipes ----------

pub const SYS_PIPE: u64 = 40;
pub const SYS_DUP2: u64 = 41;

pub fn pipe() -> Option<(u32, u32)> {
    let mut fds = [0u32; 2];
    let r = unsafe { syscall(SYS_PIPE, fds.as_mut_ptr() as u64, 0, 0) };
    if r == 0 { Some((fds[0], fds[1])) } else { None }
}

pub fn dup2(old: u32, new: u32) -> bool {
    let r = unsafe { syscall(SYS_DUP2, old as u64, new as u64, 0) };
    r == new as u64
}

// ---------- mmap ----------

pub const SYS_MMAP: u64 = 25;
pub const SYS_MUNMAP: u64 = 26;

#[inline(always)]
pub fn mmap(size: usize) -> u64 {
    unsafe { syscall(SYS_MMAP, size as u64, 0, 0) }
}

// ---------- Окна ----------

pub const WIN_BUF_VADDR: u64 = 0x0100_0000;

#[inline(always)]
pub fn create_window(w: u64, h: u64, title: &[u8]) -> u64 {
    unsafe { syscall(20, w, h, title.as_ptr() as u64) }
}

#[inline(always)]
pub fn destroy_window(id: u64) -> u64 { unsafe { syscall(21, id, 0, 0) } }

#[inline(always)]
pub fn present(id: u64) -> u64 { unsafe { syscall(22, id, 0, 0) } }

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
pub fn spawn_calc() -> u64 { unsafe { syscall(24, 0, 0, 0) } }

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

// ---------- Прочие syscalls ----------

pub const SYS_TIME_MS: u64 = 27;
pub const SYS_YIELD: u64 = 28;
pub const SYS_POLL_KEY: u64 = 29;
pub const SYS_STAT: u64 = 30;
pub const SYS_MKDIR: u64 = 31;
pub const SYS_UNLINK: u64 = 32;
pub const SYS_RENAME: u64 = 33;

#[inline(always)]
pub fn time_ms() -> u64 { unsafe { syscall(SYS_TIME_MS, 0, 0, 0) } }

#[inline(always)]
pub fn yield_now() { unsafe { syscall(SYS_YIELD, 0, 0, 0); } }

#[inline(always)]
pub fn poll_key() -> u64 { unsafe { syscall(SYS_POLL_KEY, 0, 0, 0) } }

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Stat {
    pub size: u64,
    pub is_dir: u32,
    pub _pad: u32,
}

#[inline(always)]
pub fn stat(path: &str) -> Option<Stat> {
    let mut s = Stat::default();
    let r = unsafe {
        syscall(SYS_STAT, path.as_ptr() as u64, path.len() as u64, &mut s as *mut _ as u64)
    };
    if r == 0 { Some(s) } else { None }
}

#[inline(always)]
pub fn mkdir(path: &str) -> bool {
    unsafe { syscall(SYS_MKDIR, path.as_ptr() as u64, path.len() as u64, 0) == 0 }
}

#[inline(always)]
pub fn unlink(path: &str) -> bool {
    unsafe { syscall(SYS_UNLINK, path.as_ptr() as u64, path.len() as u64, 0) == 0 }
}

#[inline(always)]
pub fn rename(old: &str, new_name: &str) -> bool {
    unsafe {
        syscall4(
            SYS_RENAME,
            old.as_ptr() as u64,
            old.len() as u64,
            new_name.as_ptr() as u64,
            new_name.len() as u64,
        ) == 0
    }
}

// ---------- Directory listing ----------

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DirEntry {
    pub name: [u8; 64],
    pub is_dir: u8,
    pub _pad: [u8; 7],
    pub size: u64,
}

impl DirEntry {
    pub fn name_str(&self) -> &str {
        let end = self.name.iter().position(|&b| b == 0).unwrap_or(64);
        core::str::from_utf8(&self.name[..end]).unwrap_or("?")
    }
}

pub fn list(path: &str, max: usize) -> alloc::vec::Vec<DirEntry> {
    let mut buf: alloc::vec::Vec<DirEntry> = alloc::vec::Vec::with_capacity(max);
    unsafe { buf.set_len(max); }
    let n = unsafe {
        syscall4(38, path.as_ptr() as u64, path.len() as u64, buf.as_mut_ptr() as u64, max as u64)
    };
    if n == u64::MAX { unsafe { buf.set_len(0); } return alloc::vec::Vec::new(); }
    unsafe { buf.set_len(n as usize); }
    buf
}

// ---------- Process list ----------

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ProcEntry {
    pub id: u64,
    pub state: u8,
    pub _pad: [u8; 7],
    pub name: [u8; 32],
}

impl ProcEntry {
    pub fn name_str(&self) -> &str {
        let end = self.name.iter().position(|&b| b == 0).unwrap_or(32);
        core::str::from_utf8(&self.name[..end]).unwrap_or("?")
    }
    pub fn state_str(&self) -> &'static str {
        match self.state {
            0 => "ready",
            1 => "running",
            2 => "sleeping",
            3 => "finished",
            _ => "?",
        }
    }
}

pub fn proc_list(max: usize) -> alloc::vec::Vec<ProcEntry> {
    let mut buf: alloc::vec::Vec<ProcEntry> = alloc::vec::Vec::with_capacity(max);
    unsafe { buf.set_len(max); }
    let n = unsafe { syscall(39, buf.as_mut_ptr() as u64, max as u64, 0) };
    if n == u64::MAX || n == 0 {
        unsafe { buf.set_len(0); }
        return alloc::vec::Vec::new();
    }
    unsafe { buf.set_len(n as usize); }
    buf
}

// ---------- GlobalAlloc ----------

const POOL_CHUNK: usize = 64 * 1024;

static POOL_BASE: AtomicUsize = AtomicUsize::new(0);
static POOL_SIZE: AtomicUsize = AtomicUsize::new(0);
static POOL_OFFSET: AtomicUsize = AtomicUsize::new(0);
static ALLOC_LOCK: AtomicBool = AtomicBool::new(false);

struct BumpAlloc;

unsafe fn alloc_lock() {
    while ALLOC_LOCK
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
}

unsafe fn alloc_unlock() { ALLOC_LOCK.store(false, Ordering::Release); }

unsafe fn pool_new(size: usize) -> bool {
    let addr = mmap(size);
    if addr == u64::MAX { return false; }
    POOL_BASE.store(addr as usize, Ordering::Relaxed);
    POOL_SIZE.store(size, Ordering::Relaxed);
    POOL_OFFSET.store(0, Ordering::Relaxed);
    true
}

unsafe fn alloc_impl(layout: Layout) -> *mut u8 {
    let align = layout.align().max(16);
    let size = layout.size();
    if POOL_BASE.load(Ordering::Relaxed) == 0 {
        let chunk = size.max(POOL_CHUNK);
        if !pool_new(chunk) { return core::ptr::null_mut(); }
    }
    loop {
        let base = POOL_BASE.load(Ordering::Relaxed);
        let pool_size = POOL_SIZE.load(Ordering::Relaxed);
        let offset = POOL_OFFSET.load(Ordering::Relaxed);
        let aligned = (offset + align - 1) & !(align - 1);
        let new_offset = aligned + size;
        if new_offset <= pool_size {
            POOL_OFFSET.store(new_offset, Ordering::Relaxed);
            return (base + aligned) as *mut u8;
        }
        let chunk = size.max(POOL_CHUNK);
        if !pool_new(chunk) { return core::ptr::null_mut(); }
    }
}

unsafe impl GlobalAlloc for BumpAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        alloc_lock();
        let result = alloc_impl(layout);
        alloc_unlock();
        result
    }
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {}
}

#[global_allocator]
static ALLOC: BumpAlloc = BumpAlloc;

#[alloc_error_handler]
fn on_alloc_error(_layout: Layout) -> ! {
    write(b"[user] OOM\n");
    exit(127);
}

// ---------- argv helpers ----------

pub unsafe fn collect_args(argc: u64, argv: *const *const u8) -> alloc::vec::Vec<alloc::string::String> {
    let mut v = alloc::vec::Vec::new();
    for i in 0..argc as usize {
        let p = *argv.add(i);
        if p.is_null() { break; }
        let mut buf: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
        for j in 0..1024 {
            let b = *p.add(j);
            if b == 0 { break; }
            buf.push(b);
        }
        if let Ok(s) = alloc::string::String::from_utf8(buf) { v.push(s); }
    }
    v
}