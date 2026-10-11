//! Syscalls через `int 0x80`.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::arch::global_asm;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::fd::{self, FdKind};
use crate::pipe;

pub const SYS_EXIT: u64 = 0;
pub const SYS_WRITE: u64 = 1;
pub const SYS_SLEEP_MS: u64 = 2;
pub const SYS_GET_TICKS: u64 = 3;
pub const SYS_GET_PID: u64 = 4;
pub const SYS_OPEN: u64 = 5;
pub const SYS_READ: u64 = 6;
pub const SYS_CLOSE: u64 = 7;
pub const SYS_SPAWN: u64 = 8;
pub const SYS_EXEC: u64 = 9;
pub const SYS_READ_KEY: u64 = 10;
pub const SYS_SET_FOCUS: u64 = 11;
pub const SYS_CREATE_WINDOW: u64 = 20;
pub const SYS_DESTROY_WINDOW: u64 = 21;
pub const SYS_PRESENT: u64 = 22;
pub const SYS_POLL_EVENT: u64 = 23;
pub const SYS_SPAWN_CALC: u64 = 24;
pub const SYS_MMAP: u64 = 25;
pub const SYS_MUNMAP: u64 = 26;
pub const SYS_TIME_MS: u64 = 27;
pub const SYS_YIELD: u64 = 28;
pub const SYS_POLL_KEY: u64 = 29;
pub const SYS_STAT: u64 = 30;
pub const SYS_MKDIR: u64 = 31;
pub const SYS_UNLINK: u64 = 32;
pub const SYS_RENAME: u64 = 33;
pub const SYS_WAIT: u64 = 34;
pub const SYS_KILL: u64 = 35;
pub const SYS_GETPPID: u64 = 36;
pub const SYS_EXEC_ARGV: u64 = 37;
pub const SYS_LIST: u64 = 38;
pub const SYS_PS: u64 = 39;
pub const SYS_PIPE: u64 = 40;
pub const SYS_DUP2: u64 = 41;
pub const SYS_OPEN_WRITE: u64 = 42;
pub const SYS_EXEC_REPLACE: u64 = 43;

global_asm!(
    ".globl syscall_entry",
    ".type syscall_entry, @function",
    "syscall_entry:",
    "    push rdi",
    "    push rsi",
    "    push rdx",
    "    push rcx",
    "    push r8",
    "    push r9",
    "    push r10",
    "    push r11",
    "    push rbx",
    "    push rbp",
    "    push r12",
    "    push r13",
    "    push r14",
    "    push r15",
    "    push rax",
    "    mov rdi, rsp",
    "    call syscall_dispatch",
    "    pop rax",
    "    pop r15",
    "    pop r14",
    "    pop r13",
    "    pop r12",
    "    pop rbp",
    "    pop rbx",
    "    pop r11",
    "    pop r10",
    "    pop r9",
    "    pop r8",
    "    pop rcx",
    "    pop rdx",
    "    pop rsi",
    "    pop rdi",
    "    iretq",
    ".size syscall_entry, . - syscall_entry",
);

extern "C" {
    pub fn syscall_entry();
}

const REG_RAX: usize = 0;
const REG_RCX: usize = 11;
const REG_RDX: usize = 12;
const REG_RSI: usize = 13;
const REG_RDI: usize = 14;

static MMAP_NEXT: AtomicU64 = AtomicU64::new(0x2000_0000);

const USER_ADDR_MAX: u64 = 0x0000_8000_0000_0000;

fn user_range_ok(ptr: u64, len: u64) -> bool {
    if ptr == 0 && len != 0 {
        return false;
    }
    if len == 0 {
        return ptr < USER_ADDR_MAX || ptr == 0;
    }
    if ptr >= USER_ADDR_MAX {
        return false;
    }
    match ptr.checked_add(len) {
        Some(end) => end <= USER_ADDR_MAX,
        None => false,
    }
}

unsafe fn user_slice<'a>(ptr: u64, len: u64) -> Option<&'a [u8]> {
    if !user_range_ok(ptr, len) {
        return None;
    }
    if len == 0 {
        return Some(&[]);
    }
    Some(core::slice::from_raw_parts(ptr as *const u8, len as usize))
}

unsafe fn user_slice_mut<'a>(ptr: u64, len: u64) -> Option<&'a mut [u8]> {
    if !user_range_ok(ptr, len) {
        return None;
    }
    if len == 0 {
        return Some(&mut []);
    }
    Some(core::slice::from_raw_parts_mut(ptr as *mut u8, len as usize))
}

#[repr(C)]
struct StatBuf {
    size: u64,
    is_dir: u32,
    _pad: u32,
}

#[no_mangle]
pub extern "C" fn syscall_dispatch(regs: *mut u64) {
    let nr = unsafe { *regs.add(REG_RAX) };
    let a1 = unsafe { *regs.add(REG_RDI) };
    let a2 = unsafe { *regs.add(REG_RSI) };
    let a3 = unsafe { *regs.add(REG_RDX) };
    let a4 = unsafe { *regs.add(REG_RCX) };

    let result: u64 = match nr {
        SYS_EXIT => {
            crate::log_info!("[syscall] exit(code={})", a1);
            crate::sched::exit_current_with_code(a1 as i32);
        }
        SYS_WRITE => sys_write(a1, a2, a3),
        SYS_SLEEP_MS => { crate::sched::sleep_ms(a1); 0 }
        SYS_GET_TICKS => crate::interrupts::ticks(),
        SYS_TIME_MS => (crate::interrupts::ticks() * 1000) / 18,
        SYS_GET_PID => crate::sched::current_id(),
        SYS_GETPPID => {
            let me = crate::sched::current_id();
            crate::sched::parent_of(me).unwrap_or(0)
        }
        SYS_YIELD => { crate::sched::yield_now(); 0 }
        SYS_POLL_KEY => sys_read_key(),
        SYS_OPEN => sys_open(a1, a2),
        SYS_OPEN_WRITE => sys_open_write(a1, a2, a3),
        SYS_READ => sys_read(a1, a2, a3),
        SYS_CLOSE => sys_close(a1),
        SYS_SPAWN => match crate::user::spawn_worker_elf() {
            Ok(id) => id,
            Err(e) => { crate::log_warn!("[syscall] spawn worker failed: {:?}", e); u64::MAX }
        },
        SYS_EXEC => sys_exec(a1, a2),
        SYS_EXEC_ARGV => sys_exec_argv(a1, a2, a3, a4),
        SYS_EXEC_REPLACE => {
            // ИСПРАВЛЕНИЕ: exec в текущем AS. Не возвращает значение,
            // если успешно — регистры перенастроены на новый entry,
            // syscall_entry сделает iretq прямо в новый _start.
            let name = match read_user_str(a1, a2) {
                Some(s) => s,
                None => return,
            };
            // SAFETY: regs валиден, прерывания выключены (внутри int 0x80).
            match unsafe { crate::user::exec_by_name_in_place(name, Vec::new(), regs) } {
                Ok(()) => return,
                Err(e) => {
                    crate::log_warn!("[syscall] exec_replace({}) failed: {:?}", name, e);
                    u64::MAX
                }
            }
        }
        SYS_READ_KEY => sys_read_key(),
        SYS_WAIT => sys_wait(a1),
        SYS_KILL => if crate::sched::kill(a1) { 0 } else { u64::MAX },
        SYS_SET_FOCUS => { crate::keyboard::set_user_owns(a1 != 0); 0 }
        SYS_SPAWN_CALC => match crate::user::spawn_calculator_elf() {
            Ok(id) => id,
            Err(e) => { crate::log_warn!("[syscall] spawn calc failed: {:?}", e); u64::MAX }
        },
        SYS_CREATE_WINDOW => sys_create_window(a1, a2, a3),
        SYS_DESTROY_WINDOW => if crate::win::destroy(a1) { 0 } else { u64::MAX },
        SYS_PRESENT => if crate::win::present(a1) { 0 } else { u64::MAX },
        SYS_POLL_EVENT => sys_poll_event(a1, a2),
        SYS_MMAP => sys_mmap(a1),
        SYS_MUNMAP => sys_munmap(a1, a2),
        SYS_STAT => sys_stat(a1, a2, a3),
        SYS_MKDIR => sys_mkdir(a1, a2),
        SYS_UNLINK => sys_unlink(a1, a2),
        SYS_RENAME => sys_rename(a1, a2, a3, a4),
        SYS_LIST => sys_list(a1, a2, a3, a4),
        SYS_PS => sys_ps(a1, a2),
        SYS_PIPE => sys_pipe(a1),
        SYS_DUP2 => sys_dup2(a1, a2),
        _ => { crate::log_warn!("[syscall] unknown nr={}", nr); u64::MAX }
    };

    unsafe { *regs.add(REG_RAX) = result; }
}

fn write_fd_serial(buf: u64, len: u64) -> u64 {
    if buf == 0 || len == 0 { return 0; }
    let Some(slice) = (unsafe { user_slice(buf, len) }) else { return u64::MAX };
    match core::str::from_utf8(slice) {
        Ok(s) => crate::serial_print!("{}", s),
        Err(_) => for &b in slice { crate::serial_print!("{}", b as char); },
    }
    len
}

fn sys_write(fd_no: u64, buf: u64, len: u64) -> u64 {
    if buf == 0 || len == 0 { return 0; }
    let pid = crate::sched::current_id();
    let kind = match fd::get(pid, fd_no as u32) {
        Some(k) => k,
        None => return u64::MAX,
    };
    let slice = match unsafe { user_slice(buf, len) } {
        Some(s) => s,
        None => return u64::MAX,
    };
    match kind {
        FdKind::Stdout | FdKind::Stderr => write_fd_serial(buf, len),
        FdKind::File { .. } => u64::MAX,
        FdKind::Stdin => u64::MAX,
        FdKind::PipeWrite { pipe_id } => {
            let mut written = 0usize;
            while written < slice.len() {
                match pipe::write(pipe_id, &slice[written..]) {
                    Some(0) => return written as u64,
                    Some(n) => {
                        written += n;
                        if written < slice.len() {
                            crate::sched::yield_now();
                        }
                    }
                    None => crate::sched::yield_now(),
                }
            }
            written as u64
        }
        FdKind::PipeRead { .. } => u64::MAX,
        FdKind::FileWrite { state } => fd::write_file(&state, slice) as u64,
    }
}

fn sys_read(fd_no: u64, buf: u64, len: u64) -> u64 {
    if buf == 0 || len == 0 { return 0; }
    let pid = crate::sched::current_id();
    let kind = match fd::get(pid, fd_no as u32) {
        Some(k) => k,
        None => return u64::MAX,
    };
    let out = match unsafe { user_slice_mut(buf, len) } {
        Some(s) => s,
        None => return u64::MAX,
    };
    match kind {
        FdKind::File { data, pos, .. } => {
            let avail = data.len().saturating_sub(pos);
            let n = avail.min(out.len());
            if n == 0 { return 0; }
            out[..n].copy_from_slice(&data[pos..pos + n]);
            fd::update_file_pos(pid, fd_no as u32, pos + n);
            n as u64
        }
        FdKind::PipeRead { pipe_id } => {
            let mut guard = 0u32;
            loop {
                match pipe::read(pipe_id, out) {
                    Some(n) => return n as u64,
                    None => {
                        guard += 1;
                        if !pipe::is_alive(pipe_id) {
                            return 0;
                        }
                        if guard > 1_000_000 {
                            return u64::MAX;
                        }
                        crate::sched::yield_now();
                    }
                }
            }
        }
        FdKind::Stdin => 0,
        FdKind::Stdout | FdKind::Stderr => u64::MAX,
        FdKind::PipeWrite { .. } => u64::MAX,
        FdKind::FileWrite { .. } => u64::MAX,
    }
}

fn sys_open(path_ptr: u64, path_len: u64) -> u64 {
    let path = match read_user_str(path_ptr, path_len) { Some(s) => s, None => return u64::MAX };
    let data = match crate::vfs::read(path) {
        Ok(d) => d,
        Err(_) => { crate::log_warn!("[syscall] open({}) — not found", path); return u64::MAX; }
    };
    let name = path.rsplit('/').next().unwrap_or(path).to_string();
    let pid = crate::sched::current_id();
    let fd_no = fd::alloc_fd(pid, FdKind::File { name, data, pos: 0 });
    fd_no as u64
}

fn sys_open_write(path_ptr: u64, path_len: u64, append_flag: u64) -> u64 {
    let path = match read_user_str(path_ptr, path_len) { Some(s) => s, None => return u64::MAX };
    let pid = crate::sched::current_id();
    let append = append_flag != 0;
    fd::open_write(pid, path.to_string(), append) as u64
}

fn sys_close(fd_no: u64) -> u64 {
    let pid = crate::sched::current_id();
    if fd::close(pid, fd_no as u32) { 0 } else { u64::MAX }
}

fn sys_pipe(out_ptr: u64) -> u64 {
    if out_ptr == 0 { return u64::MAX; }
    if !user_range_ok(out_ptr, 8) { return u64::MAX; }
    let pipe_id = match pipe::create() { Some(id) => id, None => return u64::MAX };
    let pid = crate::sched::current_id();
    let r = fd::alloc_fd(pid, FdKind::PipeRead { pipe_id });
    let w = fd::alloc_fd(pid, FdKind::PipeWrite { pipe_id });
    unsafe {
        let p = out_ptr as *mut u32;
        *p = r;
        *p.add(1) = w;
    }
    0
}

fn sys_dup2(old_fd: u64, new_fd: u64) -> u64 {
    let pid = crate::sched::current_id();
    if fd::dup2(pid, old_fd as u32, new_fd as u32) { new_fd } else { u64::MAX }
}

fn read_user_str(ptr: u64, len: u64) -> Option<&'static str> {
    if ptr == 0 || len == 0 { return None; }
    let slice = unsafe { user_slice(ptr, len)? };
    core::str::from_utf8(slice).ok()
}

fn read_user_cstr(ptr: u64, max: usize) -> Option<String> {
    if ptr == 0 { return None; }
    if !user_range_ok(ptr, max.min(4096) as u64) { return None; }
    let mut buf: Vec<u8> = Vec::with_capacity(max.min(256));
    unsafe {
        for i in 0..max.min(4096) {
            let b = core::ptr::read_volatile((ptr as *const u8).add(i));
            if b == 0 { break; }
            buf.push(b);
        }
    }
    String::from_utf8(buf).ok()
}

fn sys_exec(name_ptr: u64, name_len: u64) -> u64 {
    let name = match read_user_str(name_ptr, name_len) { Some(s) => s, None => return u64::MAX };
    match crate::user::exec_by_name(name) {
        Ok(id) => id,
        Err(e) => { crate::log_warn!("[syscall] exec({}) failed: {:?}", name, e); u64::MAX }
    }
}

fn sys_exec_argv(name_ptr: u64, name_len: u64, argv_ptrs: u64, argc: u64) -> u64 {
    let name = match read_user_str(name_ptr, name_len) { Some(s) => s, None => return u64::MAX };
    let argc = argc.min(64);
    if !user_range_ok(argv_ptrs, argc * 8) { return u64::MAX; }
    let mut args: Vec<String> = Vec::new();
    for i in 0..argc {
        let p = unsafe { core::ptr::read_volatile((argv_ptrs + i * 8) as *const u64) };
        if p == 0 { break; }
        let s = match read_user_cstr(p, 256) { Some(s) => s, None => return u64::MAX };
        args.push(s);
    }
    match crate::user::exec_by_name_args(name, args) {
        Ok(id) => id,
        Err(e) => { crate::log_warn!("[syscall] exec_argv({}) failed: {:?}", name, e); u64::MAX }
    }
}

fn sys_wait(pid: u64) -> u64 {
    if pid == 0 { return u64::MAX; }
    match crate::sched::wait_for(pid) {
        Some(code) => code as i64 as u64,
        None => u64::MAX,
    }
}

fn sys_read_key() -> u64 {
    use crate::keyboard::Key;
    match crate::keyboard::pop() {
        Some(Key::Char(c)) => c as u64,
        Some(Key::Enter) => 0x0A,
        Some(Key::Backspace) => 0x08,
        Some(Key::Escape) => 0x1B,
        Some(Key::Tab) => 0x09,
        Some(Key::Up) => 0x80,
        Some(Key::Down) => 0x81,
        Some(Key::Left) => 0x82,
        Some(Key::Right) => 0x83,
        Some(Key::Home) => 0x84,
        Some(Key::End) => 0x85,
        Some(Key::PageUp) => 0x86,
        Some(Key::PageDown) => 0x87,
        Some(Key::Delete) => 0x88,
        Some(Key::F(n)) if n >= 1 && n <= 12 => 0x90 + (n as u64) - 1,
        Some(_) => u64::MAX,
        None => u64::MAX,
    }
}

fn sys_create_window(w: u64, h: u64, title_ptr: u64) -> u64 {
    use x86_64::structures::paging::{FrameAllocator, PageTableFlags};
    use x86_64::VirtAddr;

    let pid = crate::sched::current_id();
    let phys_offset_u64 = crate::memory::PHYS_OFFSET.load(Ordering::Acquire);
    if phys_offset_u64 == 0 { return u64::MAX; }
    let phys_offset = VirtAddr::new(phys_offset_u64);

    let title = if title_ptr != 0 {
        match read_user_cstr(title_ptr, 64) { Some(s) => s, None => String::from("window") }
    } else {
        String::from("window")
    };

    let buf_len = match (w as usize).checked_mul(h as usize).and_then(|v| v.checked_mul(4)) {
        Some(v) => v,
        None => return u64::MAX,
    };
    let pages_needed = match buf_len.checked_add(4095) {
        Some(v) => v / 4096,
        None => return u64::MAX,
    };

    let mut guard = crate::memory::FRAME_ALLOCATOR.lock();
    let alloc = match guard.as_mut() { Some(a) => a, None => return u64::MAX };

    let flags = PageTableFlags::PRESENT
        | PageTableFlags::WRITABLE
        | PageTableFlags::USER_ACCESSIBLE
        | PageTableFlags::NO_EXECUTE;

    for i in 0..pages_needed {
        let frame = match alloc.allocate_frame() { Some(f) => f, None => return u64::MAX };
        let virt = crate::win::WIN_BUF_VADDR + (i * 4096) as u64;
        let ok = unsafe { crate::memory::map_current_as(phys_offset, virt, frame, flags, alloc) };
        if !ok { return u64::MAX; }
    }
    drop(guard);

    match crate::win::alloc_window(pid, w as u32, h as u32, title, crate::win::WIN_BUF_VADDR) {
        Some(v) => v,
        None => u64::MAX,
    }
}

fn sys_poll_event(win_id: u64, out_ptr: u64) -> u64 {
    if out_ptr == 0 { return 0; }
    let sz = core::mem::size_of::<crate::win::WinEvent>() as u64;
    if !user_range_ok(out_ptr, sz) { return 0; }
    match crate::win::poll_event(win_id) {
        Some(ev) => {
            unsafe { core::ptr::write_volatile(out_ptr as *mut crate::win::WinEvent, ev); }
            1
        }
        None => 0,
    }
}

fn sys_mmap(size: u64) -> u64 {
    use x86_64::structures::paging::{FrameAllocator, PageTableFlags};
    use x86_64::VirtAddr;

    if size == 0 { return u64::MAX; }
    let phys_offset_u64 = crate::memory::PHYS_OFFSET.load(Ordering::Acquire);
    if phys_offset_u64 == 0 { return u64::MAX; }
    let phys_offset = VirtAddr::new(phys_offset_u64);

    let total = match size.checked_add(4095).map(|v| v & !4095u64) {
        Some(t) => t,
        None => return u64::MAX,
    };
    let pages = total / 4096;

    let base = MMAP_NEXT.fetch_add(total, Ordering::AcqRel);
    if base.checked_add(total).map(|e| e > USER_ADDR_MAX).unwrap_or(true) {
        return u64::MAX;
    }

    let mut guard = crate::memory::FRAME_ALLOCATOR.lock();
    let alloc = match guard.as_mut() { Some(a) => a, None => return u64::MAX };

    let flags = PageTableFlags::PRESENT
        | PageTableFlags::WRITABLE
        | PageTableFlags::USER_ACCESSIBLE
        | PageTableFlags::NO_EXECUTE;

    for i in 0..pages {
        let frame = match alloc.allocate_frame() { Some(f) => f, None => return u64::MAX };
        let virt = base + i * 4096;
        let ok = unsafe { crate::memory::map_current_as(phys_offset, virt, frame, flags, alloc) };
        if !ok { return u64::MAX; }
        unsafe { core::ptr::write_bytes(virt as *mut u8, 0, 4096); }
    }
    base
}

fn sys_munmap(_addr: u64, _size: u64) -> u64 { 0 }

fn sys_stat(path_ptr: u64, path_len: u64, out_ptr: u64) -> u64 {
    if out_ptr == 0 { return u64::MAX; }
    let sz = core::mem::size_of::<StatBuf>() as u64;
    if !user_range_ok(out_ptr, sz) { return u64::MAX; }
    let path = match read_user_str(path_ptr, path_len) { Some(s) => s, None => return u64::MAX };
    let (size, is_dir) = if path.starts_with("C:") {
        match crate::vfs::fat32_stat_path(path) { Some(v) => v, None => return u64::MAX }
    } else {
        match crate::vfs::ramfs_stat(path) { Some(v) => v, None => return u64::MAX }
    };
    let buf = StatBuf { size, is_dir: if is_dir { 1 } else { 0 }, _pad: 0 };
    unsafe { core::ptr::write_volatile(out_ptr as *mut StatBuf, buf); }
    0
}

fn sys_mkdir(path_ptr: u64, path_len: u64) -> u64 {
    let path = match read_user_str(path_ptr, path_len) { Some(s) => s, None => return u64::MAX };
    let ok = if path.starts_with("C:") {
        crate::vfs::fat32_mkdir_path(path)
    } else {
        let parent = crate::fs::parent_path(path);
        let name = path.rsplit('/').next().unwrap_or(path);
        crate::vfs::ramfs_mkdir(&parent, name)
    };
    if ok { 0 } else { u64::MAX }
}

fn sys_unlink(path_ptr: u64, path_len: u64) -> u64 {
    let path = match read_user_str(path_ptr, path_len) { Some(s) => s, None => return u64::MAX };
    let ok = if path.starts_with("C:") {
        crate::vfs::fat32_remove_path(path)
    } else {
        crate::vfs::ramfs_remove(path)
    };
    if ok { 0 } else { u64::MAX }
}

fn sys_rename(old_ptr: u64, old_len: u64, new_ptr: u64, new_len: u64) -> u64 {
    let old = match read_user_str(old_ptr, old_len) { Some(s) => s, None => return u64::MAX };
    let new_name = match read_user_str(new_ptr, new_len) { Some(s) => s, None => return u64::MAX };
    let ok = if old.starts_with("C:") {
        crate::vfs::fat32_rename_path(old, new_name)
    } else {
        crate::vfs::ramfs_rename(old, new_name)
    };
    if ok { 0 } else { u64::MAX }
}

const DIR_ENTRY_SIZE: u64 = 80;
const PROC_ENTRY_SIZE: u64 = 48;

fn sys_list(path_ptr: u64, path_len: u64, buf_ptr: u64, buf_count: u64) -> u64 {
    let path = match read_user_str(path_ptr, path_len) { Some(s) => s, None => return u64::MAX };
    let entries = crate::vfs::list(path).ok().unwrap_or_default();
    if buf_ptr == 0 || buf_count == 0 {
        return entries.len() as u64;
    }
    let n = entries.len().min(buf_count as usize);
    let need = (n as u64) * DIR_ENTRY_SIZE;
    if !user_range_ok(buf_ptr, need) { return u64::MAX; }

    for (i, e) in entries.iter().take(n).enumerate() {
        let ep = (buf_ptr + (i as u64) * DIR_ENTRY_SIZE) as *mut u8;
        unsafe {
            let name_bytes = e.name.as_bytes();
            let copy_len = name_bytes.len().min(63);
            for j in 0..64usize {
                *ep.add(j) = if j < copy_len { name_bytes[j] } else { 0 };
            }
            *ep.add(64) = if e.is_dir { 1 } else { 0 };
            for j in 65..72 { *ep.add(j) = 0; }
            let szb = (e.size as u64).to_le_bytes();
            for j in 0..8 { *ep.add(72 + j) = szb[j]; }
        }
    }
    n as u64
}

fn sys_ps(buf_ptr: u64, buf_count: u64) -> u64 {
    if buf_ptr == 0 { return 0; }
    let threads = crate::sched::list_threads_snapshot();
    let n = threads.len().min(buf_count as usize);
    let need = (n as u64) * PROC_ENTRY_SIZE;
    if !user_range_ok(buf_ptr, need) { return u64::MAX; }
    for (i, t) in threads.iter().take(n).enumerate() {
        let ep = (buf_ptr + (i as u64) * PROC_ENTRY_SIZE) as *mut u8;
        unsafe {
            let idb = t.id.to_le_bytes();
            for j in 0..8 { *ep.add(j) = idb[j]; }
            *ep.add(8) = t.state;
            for j in 9..16 { *ep.add(j) = 0; }
            for j in 0..32 { *ep.add(16 + j) = t.name[j]; }
        }
    }
    n as u64
}