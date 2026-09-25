//! Syscalls через `int 0x80`.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::arch::global_asm;

pub const SYS_EXIT: u64 = 0;
pub const SYS_WRITE: u64 = 1;
pub const SYS_SLEEP_MS: u64 = 2;
pub const SYS_GET_TICKS: u64 = 3;
pub const SYS_GET_PID: u64 = 4;
pub const SYS_OPEN: u64 = 5;
pub const SYS_READ: u64 = 6;
pub const SYS_CLOSE: u64 = 7;
pub const SYS_SPAWN: u64 = 8;
pub const SYS_SPAWN_CALC: u64 = 24;
pub const SYS_CREATE_WINDOW: u64 = 20;
pub const SYS_DESTROY_WINDOW: u64 = 21;
pub const SYS_PRESENT: u64 = 22;
pub const SYS_POLL_EVENT: u64 = 23;

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
const REG_RDX: usize = 12;
const REG_RSI: usize = 13;
const REG_RDI: usize = 14;

struct FileHandle {
    #[allow(dead_code)]
    name: String,
    data: Vec<u8>,
    pos: usize,
}

static FD_TABLE: spin::Mutex<Vec<Option<FileHandle>>> = spin::Mutex::new(Vec::new());

#[no_mangle]
pub extern "C" fn syscall_dispatch(regs: *mut u64) {
    let nr = unsafe { *regs.add(REG_RAX) };
    let a1 = unsafe { *regs.add(REG_RDI) };
    let a2 = unsafe { *regs.add(REG_RSI) };
    let a3 = unsafe { *regs.add(REG_RDX) };

    let result: u64 = match nr {
        SYS_EXIT => {
            crate::serial_println!("[syscall] exit(code={})", a1);
            crate::sched::exit_current();
        }
        SYS_WRITE => write_fd(a1, a2, a3),
        SYS_SLEEP_MS => { crate::sched::sleep_ms(a1); 0 }
        SYS_GET_TICKS => crate::interrupts::ticks(),
        SYS_GET_PID => crate::sched::current_id(),
        SYS_OPEN => sys_open(a1, a2),
        SYS_READ => sys_read(a1, a2, a3),
        SYS_CLOSE => sys_close(a1),
        SYS_SPAWN => match crate::user::spawn_worker_elf() {
            Some(id) => id, None => u64::MAX,
        },
        SYS_SPAWN_CALC => match crate::user::spawn_calculator_elf() {
            Some(id) => id, None => u64::MAX,
        },
        SYS_CREATE_WINDOW => sys_create_window(a1, a2, a3),
        SYS_DESTROY_WINDOW => if crate::win::destroy(a1) { 0 } else { u64::MAX },
        SYS_PRESENT => if crate::win::present(a1) { 0 } else { u64::MAX },
        SYS_POLL_EVENT => sys_poll_event(a1, a2),
        _ => {
            crate::serial_println!("[syscall] unknown nr={}", nr);
            u64::MAX
        }
    };

    unsafe { *regs.add(REG_RAX) = result; }
}

fn write_fd(fd: u64, buf: u64, len: u64) -> u64 {
    if buf == 0 || len == 0 { return 0; }
    let slice = unsafe { core::slice::from_raw_parts(buf as *const u8, len as usize) };
    match fd {
        1 | 2 => {
            match core::str::from_utf8(slice) {
                Ok(s) => crate::serial_print!("{}", s),
                Err(_) => {
                    for &b in slice { crate::serial_print!("{}", b as char); }
                }
            }
            0
        }
        _ => u64::MAX,
    }
}

fn sys_open(path_ptr: u64, path_len: u64) -> u64 {
    if path_ptr == 0 || path_len == 0 {
        return u64::MAX;
    }
    let slice = unsafe {
        core::slice::from_raw_parts(path_ptr as *const u8, path_len as usize)
    };
    let path = match core::str::from_utf8(slice) {
        Ok(s) => s,
        Err(_) => return u64::MAX,
    };

    let data = match crate::vfs::read(path) {
        Ok(d) => d,
        Err(_) => {
            crate::serial_println!("[syscall] open({}) — not found", path);
            return u64::MAX;
        }
    };

    let name = path.rsplit('/').next().unwrap_or(path).to_string();

    let mut table = FD_TABLE.lock();
    for (i, slot) in table.iter_mut().enumerate() {
        if slot.is_none() {
            *slot = Some(FileHandle {
                name,
                data,
                pos: 0,
            });
            return i as u64;
        }
    }
    table.push(Some(FileHandle {
        name,
        data,
        pos: 0,
    }));
    (table.len() - 1) as u64
}

fn sys_read(fd: u64, buf: u64, len: u64) -> u64 {
    if buf == 0 || len == 0 { return 0; }
    let mut table = FD_TABLE.lock();
    let slot = match table.get_mut(fd as usize).and_then(|s| s.as_mut()) {
        Some(h) => h, None => return u64::MAX,
    };
    let avail = slot.data.len().saturating_sub(slot.pos);
    let n = avail.min(len as usize);
    if n == 0 { return 0; }
    unsafe {
        core::ptr::copy_nonoverlapping(
            slot.data.as_ptr().add(slot.pos),
            buf as *mut u8,
            n,
        );
    }
    slot.pos += n;
    n as u64
}

fn sys_close(fd: u64) -> u64 {
    let mut table = FD_TABLE.lock();
    if let Some(slot) = table.get_mut(fd as usize) { *slot = None; 0 } else { u64::MAX }
}

fn sys_create_window(w: u64, h: u64, title_ptr: u64) -> u64 {
    use x86_64::structures::paging::{FrameAllocator, PageTableFlags};
    use x86_64::VirtAddr;

    let pid = crate::sched::current_id();
    let phys_offset_u64 =
        crate::memory::PHYS_OFFSET.load(core::sync::atomic::Ordering::Acquire);
    if phys_offset_u64 == 0 { return u64::MAX; }
    let phys_offset = VirtAddr::new(phys_offset_u64);

    let title = if title_ptr != 0 {
        let mut buf = [0u8; 64];
        let mut n = 0usize;
        unsafe {
            while n < 64 {
                let b = core::ptr::read_volatile((title_ptr as *const u8).add(n));
                if b == 0 { break; }
                buf[n] = b;
                n += 1;
            }
        }
        match core::str::from_utf8(&buf[..n]) {
            Ok(s) => String::from(s),
            Err(_) => String::from("window"),
        }
    } else {
        String::from("window")
    };

    let buf_len = (w as usize) * (h as usize) * 4;
    let pages_needed = (buf_len + 4095) / 4096;

    let mut guard = crate::memory::FRAME_ALLOCATOR.lock();
    let alloc = match guard.as_mut() { Some(a) => a, None => return u64::MAX };

    let flags = PageTableFlags::PRESENT
        | PageTableFlags::WRITABLE
        | PageTableFlags::USER_ACCESSIBLE;

    for i in 0..pages_needed {
        let frame = match alloc.allocate_frame() { Some(f) => f, None => return u64::MAX };
        let virt = crate::win::WIN_BUF_VADDR + (i * 4096) as u64;
        let ok = unsafe {
            crate::memory::map_current_as(phys_offset, virt, frame, flags, alloc)
        };
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
    match crate::win::poll_event(win_id) {
        Some(ev) => {
            unsafe {
                core::ptr::write_volatile(out_ptr as *mut crate::win::WinEvent, ev);
            }
            1
        }
        None => 0,
    }
}