//! Запуск Ring 3: отдельное адресное пространство, загрузка ELF, iretq.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::arch::asm;
use core::sync::atomic::{AtomicU64, Ordering};
use spin::Mutex;

use x86_64::{
    registers::control::{Cr3, Cr3Flags},
    structures::paging::{FrameAllocator, Mapper, Page, PageTableFlags, Size4KiB},
    VirtAddr,
};

use crate::{log_debug, log_info};

const USER_STACK_GUARD: u64 = 0x80_0000;
const USER_STACK_GUARD_SIZE: u64 = 4096;
const USER_STACK_SIZE: u64 = 32 * 1024;
const USER_STACK_BASE: u64 = USER_STACK_GUARD + USER_STACK_GUARD_SIZE;
const USER_STACK_TOP_ADDR: u64 = USER_STACK_BASE + USER_STACK_SIZE;

static USER_ELF: &[u8] = include_bytes!("../user.elf");
static INIT_ELF: &[u8] = include_bytes!("../init.elf");
static WORKER_ELF: &[u8] = include_bytes!("../worker.elf");
static CALC_ELF: &[u8] = include_bytes!("../calculator.elf");
static SHELL_ELF: &[u8] = include_bytes!("../shell.elf");
static ECHO_ELF: &[u8] = include_bytes!("../echo.elf");
static LS_ELF: &[u8] = include_bytes!("../ls.elf");
static CAT_ELF: &[u8] = include_bytes!("../cat.elf");
static PS_ELF: &[u8] = include_bytes!("../ps.elf");
static CP_ELF: &[u8] = include_bytes!("../cp.elf");
static WC_ELF: &[u8] = include_bytes!("../wc.elf");
static HEAD_ELF: &[u8] = include_bytes!("../head.elf");
static DMESG_ELF: &[u8] = include_bytes!("../dmesg.elf");
static KILL_ELF: &[u8] = include_bytes!("../kill.elf");
static UNAME_ELF: &[u8] = include_bytes!("../uname.elf");
static WHICH_ELF: &[u8] = include_bytes!("../which.elf");
static ENV_ELF: &[u8] = include_bytes!("../env.elf");
static VIEWER_ELF: &[u8] = include_bytes!("../viewer.elf");
static PLAY_ELF: &[u8] = include_bytes!("../play.elf");
static PING_ELF: &[u8] = include_bytes!("../ping.elf");

static USER_ENTRY: AtomicU64 = AtomicU64::new(0);
static USER_STACK_TOP: AtomicU64 = AtomicU64::new(0);
static INIT_ENTRY: AtomicU64 = AtomicU64::new(0);
static INIT_STACK_TOP: AtomicU64 = AtomicU64::new(0);
static WORKER_ENTRY: AtomicU64 = AtomicU64::new(0);
static WORKER_STACK_TOP: AtomicU64 = AtomicU64::new(0);
static CALC_ENTRY: AtomicU64 = AtomicU64::new(0);
static CALC_STACK_TOP: AtomicU64 = AtomicU64::new(0);
static SHELL_ENTRY: AtomicU64 = AtomicU64::new(0);
static SHELL_STACK_TOP: AtomicU64 = AtomicU64::new(0);
static ECHO_ENTRY: AtomicU64 = AtomicU64::new(0);
static ECHO_STACK_TOP: AtomicU64 = AtomicU64::new(0);
static LS_ENTRY: AtomicU64 = AtomicU64::new(0);
static LS_STACK_TOP: AtomicU64 = AtomicU64::new(0);
static CAT_ENTRY: AtomicU64 = AtomicU64::new(0);
static CAT_STACK_TOP: AtomicU64 = AtomicU64::new(0);
static PS_ENTRY: AtomicU64 = AtomicU64::new(0);
static PS_STACK_TOP: AtomicU64 = AtomicU64::new(0);
static CP_ENTRY: AtomicU64 = AtomicU64::new(0);
static CP_STACK_TOP: AtomicU64 = AtomicU64::new(0);
static WC_ENTRY: AtomicU64 = AtomicU64::new(0);
static WC_STACK_TOP: AtomicU64 = AtomicU64::new(0);
static HEAD_ENTRY: AtomicU64 = AtomicU64::new(0);
static HEAD_STACK_TOP: AtomicU64 = AtomicU64::new(0);
static DMESG_ENTRY: AtomicU64 = AtomicU64::new(0);
static DMESG_STACK_TOP: AtomicU64 = AtomicU64::new(0);
static KILL_ENTRY: AtomicU64 = AtomicU64::new(0);
static KILL_STACK_TOP: AtomicU64 = AtomicU64::new(0);
static UNAME_ENTRY: AtomicU64 = AtomicU64::new(0);
static UNAME_STACK_TOP: AtomicU64 = AtomicU64::new(0);
static WHICH_ENTRY: AtomicU64 = AtomicU64::new(0);
static WHICH_STACK_TOP: AtomicU64 = AtomicU64::new(0);
static ENV_ENTRY: AtomicU64 = AtomicU64::new(0);
static ENV_STACK_TOP: AtomicU64 = AtomicU64::new(0);
static VIEWER_ENTRY: AtomicU64 = AtomicU64::new(0);
static VIEWER_STACK_TOP: AtomicU64 = AtomicU64::new(0);
static PLAY_ENTRY: AtomicU64 = AtomicU64::new(0);
static PLAY_STACK_TOP: AtomicU64 = AtomicU64::new(0);
static PING_ENTRY: AtomicU64 = AtomicU64::new(0);
static PING_STACK_TOP: AtomicU64 = AtomicU64::new(0);

static PENDING_ARGS: Mutex<Vec<(u64, Vec<String>)>> = Mutex::new(Vec::new());

#[repr(C)]
#[derive(Clone, Copy)]
struct ForkReturn {
    rip: u64,
    rflags: u64,
    rsp: u64,
    rax: u64,
    rbx: u64,
    rbp: u64,
    r12: u64,
    r13: u64,
    r14: u64,
    r15: u64,
}

static PENDING_FORK_RETURNS: Mutex<BTreeMap<u64, ForkReturn>> = Mutex::new(BTreeMap::new());

#[derive(Debug)]
pub enum SpawnError {
    NoPhysOffset,
    NoFrameAllocator,
    NoAddressSpace,
    Elf(crate::elf::ElfError),
    MapFailed,
    UnknownProgram,
}

pub type SpawnResult<T> = Result<T, SpawnError>;

fn spawn_elf(
    elf: &[u8],
    name: &'static str,
    entry_slot: &AtomicU64,
    stack_slot: &AtomicU64,
    entry_fn: extern "C" fn() -> !,
    args: Vec<String>,
) -> SpawnResult<u64> {
    let phys_offset_u64 = crate::memory::PHYS_OFFSET.load(Ordering::Acquire);
    if phys_offset_u64 == 0 { return Err(SpawnError::NoPhysOffset); }
    let phys_offset = VirtAddr::new(phys_offset_u64);

    let mut guard = crate::memory::FRAME_ALLOCATOR.lock();
    let alloc = guard.as_mut().ok_or(SpawnError::NoFrameAllocator)?;
    let as_ = unsafe {
        crate::memory::AddressSpace::new_user(phys_offset, alloc)
            .ok_or(SpawnError::NoAddressSpace)?
    };

    let loaded = unsafe {
        x86_64::instructions::interrupts::without_interrupts(
            || -> SpawnResult<crate::elf::LoadedElf> {
                let (saved_frame, saved_flags) = Cr3::read();
                Cr3::write(as_.pml4_frame, Cr3Flags::empty());

                let mut mapper = as_.mapper(phys_offset);
                let loaded = match crate::elf::load(elf, &mut mapper, alloc) {
                    Ok(l) => l,
                    Err(e) => {
                        Cr3::write(saved_frame, saved_flags);
                        return Err(SpawnError::Elf(e));
                    }
                };

                let p = loaded.entry as *const u8;
                let mut buf = [0u8; 16];
                for i in 0..16 {
                    buf[i] = core::ptr::read_volatile(p.add(i));
                }
                log_debug!("[elf] bytes @ entry({:#x}): {:02x?}", loaded.entry, buf);

                let stack_flags = PageTableFlags::PRESENT
                    | PageTableFlags::WRITABLE
                    | PageTableFlags::USER_ACCESSIBLE
                    | PageTableFlags::NO_EXECUTE;

                let map_res = map_user_region(
                    &mut mapper, alloc,
                    USER_STACK_BASE, USER_STACK_SIZE,
                    stack_flags,
                );
                if let Err(e) = map_res {
                    Cr3::write(saved_frame, saved_flags);
                    return Err(e);
                }

                Cr3::write(saved_frame, saved_flags);
                Ok(loaded)
            },
        )
    }?;

    let pml4_phys = as_.pml4_frame.start_address().as_u64();
    drop(guard);

    entry_slot.store(loaded.entry, Ordering::Release);
    stack_slot.store(USER_STACK_TOP_ADDR, Ordering::Release);

    let parent = crate::sched::current_id();

    let id = x86_64::instructions::interrupts::without_interrupts(|| {
        let id = crate::sched::spawn_with_as(name, entry_fn, pml4_phys);
        PENDING_ARGS.lock().push((id, args));
        id
    });

    crate::fd::fork_from(parent, id);
    Ok(id)
}

fn take_args_for(id: u64) -> Vec<String> {
    let mut p = PENDING_ARGS.lock();
    let pos = p.iter().position(|(tid, _)| *tid == id);
    match pos {
        Some(i) => p.remove(i).1,
        None => Vec::new(),
    }
}

pub unsafe fn spawn_user_test(_phys_offset: VirtAddr) -> SpawnResult<u64> {
    let id = spawn_elf(USER_ELF, "user", &USER_ENTRY, &USER_STACK_TOP, user_entry, Vec::new())?;
    log_info!("[user] ELF loaded: entry={:#x}", USER_ENTRY.load(Ordering::Acquire));
    Ok(id)
}

extern "C" fn user_entry() -> ! {
    x86_64::instructions::interrupts::enable();
    let id = crate::sched::current_id();
    let args = take_args_for(id);
    let entry = USER_ENTRY.load(Ordering::Acquire);
    let stack_top = USER_STACK_TOP.load(Ordering::Acquire);
    unsafe { enter_user(entry, stack_top, &args) }
}

pub unsafe fn spawn_init_elf(_phys_offset: VirtAddr) -> SpawnResult<u64> {
    let id = spawn_elf(
        INIT_ELF, "init",
        &INIT_ENTRY, &INIT_STACK_TOP,
        init_entry, Vec::new(),
    )?;
    log_info!("[user] init loaded, entry={:#x}", INIT_ENTRY.load(Ordering::Acquire));
    Ok(id)
}

extern "C" fn init_entry() -> ! {
    x86_64::instructions::interrupts::enable();
    let id = crate::sched::current_id();
    let args = take_args_for(id);
    let entry = INIT_ENTRY.load(Ordering::Acquire);
    let stack_top = INIT_STACK_TOP.load(Ordering::Acquire);
    unsafe { enter_user(entry, stack_top, &args) }
}

pub fn spawn_worker_elf() -> SpawnResult<u64> {
    spawn_elf(WORKER_ELF, "worker", &WORKER_ENTRY, &WORKER_STACK_TOP, worker_entry, Vec::new())
}

extern "C" fn worker_entry() -> ! {
    x86_64::instructions::interrupts::enable();
    let id = crate::sched::current_id();
    let args = take_args_for(id);
    let entry = WORKER_ENTRY.load(Ordering::Acquire);
    let stack_top = WORKER_STACK_TOP.load(Ordering::Acquire);
    unsafe { enter_user(entry, stack_top, &args) }
}

pub fn spawn_calculator_elf() -> SpawnResult<u64> {
    spawn_elf(CALC_ELF, "calc", &CALC_ENTRY, &CALC_STACK_TOP, calc_entry, Vec::new())
}

extern "C" fn calc_entry() -> ! {
    x86_64::instructions::interrupts::enable();
    let id = crate::sched::current_id();
    let args = take_args_for(id);
    let entry = CALC_ENTRY.load(Ordering::Acquire);
    let stack_top = CALC_STACK_TOP.load(Ordering::Acquire);
    unsafe { enter_user(entry, stack_top, &args) }
}

pub fn spawn_shell_elf() -> SpawnResult<u64> {
    spawn_elf(SHELL_ELF, "shell", &SHELL_ENTRY, &SHELL_STACK_TOP, shell_entry, Vec::new())
}

extern "C" fn shell_entry() -> ! {
    x86_64::instructions::interrupts::enable();
    let id = crate::sched::current_id();
    let args = take_args_for(id);
    let entry = SHELL_ENTRY.load(Ordering::Acquire);
    let stack_top = SHELL_STACK_TOP.load(Ordering::Acquire);
    unsafe { enter_user(entry, stack_top, &args) }
}

pub fn spawn_echo_elf(args: Vec<String>) -> SpawnResult<u64> {
    spawn_elf(ECHO_ELF, "echo", &ECHO_ENTRY, &ECHO_STACK_TOP, echo_entry, args)
}

extern "C" fn echo_entry() -> ! {
    x86_64::instructions::interrupts::enable();
    let id = crate::sched::current_id();
    let args = take_args_for(id);
    let entry = ECHO_ENTRY.load(Ordering::Acquire);
    let stack_top = ECHO_STACK_TOP.load(Ordering::Acquire);
    unsafe { enter_user(entry, stack_top, &args) }
}

pub fn spawn_ls_elf(args: Vec<String>) -> SpawnResult<u64> {
    spawn_elf(LS_ELF, "ls", &LS_ENTRY, &LS_STACK_TOP, ls_entry, args)
}

extern "C" fn ls_entry() -> ! {
    x86_64::instructions::interrupts::enable();
    let id = crate::sched::current_id();
    let args = take_args_for(id);
    let entry = LS_ENTRY.load(Ordering::Acquire);
    let stack_top = LS_STACK_TOP.load(Ordering::Acquire);
    unsafe { enter_user(entry, stack_top, &args) }
}

pub fn spawn_cat_elf(args: Vec<String>) -> SpawnResult<u64> {
    spawn_elf(CAT_ELF, "cat", &CAT_ENTRY, &CAT_STACK_TOP, cat_entry, args)
}

extern "C" fn cat_entry() -> ! {
    x86_64::instructions::interrupts::enable();
    let id = crate::sched::current_id();
    let args = take_args_for(id);
    let entry = CAT_ENTRY.load(Ordering::Acquire);
    let stack_top = CAT_STACK_TOP.load(Ordering::Acquire);
    unsafe { enter_user(entry, stack_top, &args) }
}

pub fn spawn_ps_elf() -> SpawnResult<u64> {
    spawn_elf(PS_ELF, "ps", &PS_ENTRY, &PS_STACK_TOP, ps_entry, Vec::new())
}

extern "C" fn ps_entry() -> ! {
    x86_64::instructions::interrupts::enable();
    let id = crate::sched::current_id();
    let args = take_args_for(id);
    let entry = PS_ENTRY.load(Ordering::Acquire);
    let stack_top = PS_STACK_TOP.load(Ordering::Acquire);
    unsafe { enter_user(entry, stack_top, &args) }
}

pub fn spawn_cp_elf(args: Vec<String>) -> SpawnResult<u64> {
    spawn_elf(CP_ELF, "cp", &CP_ENTRY, &CP_STACK_TOP, cp_entry, args)
}

extern "C" fn cp_entry() -> ! {
    x86_64::instructions::interrupts::enable();
    let id = crate::sched::current_id();
    let args = take_args_for(id);
    let entry = CP_ENTRY.load(Ordering::Acquire);
    let stack_top = CP_STACK_TOP.load(Ordering::Acquire);
    unsafe { enter_user(entry, stack_top, &args) }
}

pub fn spawn_wc_elf(args: Vec<String>) -> SpawnResult<u64> {
    spawn_elf(WC_ELF, "wc", &WC_ENTRY, &WC_STACK_TOP, wc_entry, args)
}

extern "C" fn wc_entry() -> ! {
    x86_64::instructions::interrupts::enable();
    let id = crate::sched::current_id();
    let args = take_args_for(id);
    let entry = WC_ENTRY.load(Ordering::Acquire);
    let stack_top = WC_STACK_TOP.load(Ordering::Acquire);
    unsafe { enter_user(entry, stack_top, &args) }
}

pub fn spawn_head_elf(args: Vec<String>) -> SpawnResult<u64> {
    spawn_elf(HEAD_ELF, "head", &HEAD_ENTRY, &HEAD_STACK_TOP, head_entry, args)
}

extern "C" fn head_entry() -> ! {
    x86_64::instructions::interrupts::enable();
    let id = crate::sched::current_id();
    let args = take_args_for(id);
    let entry = HEAD_ENTRY.load(Ordering::Acquire);
    let stack_top = HEAD_STACK_TOP.load(Ordering::Acquire);
    unsafe { enter_user(entry, stack_top, &args) }
}

pub fn spawn_dmesg_elf(args: Vec<String>) -> SpawnResult<u64> {
    spawn_elf(DMESG_ELF, "dmesg", &DMESG_ENTRY, &DMESG_STACK_TOP, dmesg_entry, args)
}

extern "C" fn dmesg_entry() -> ! {
    x86_64::instructions::interrupts::enable();
    let id = crate::sched::current_id();
    let args = take_args_for(id);
    let entry = DMESG_ENTRY.load(Ordering::Acquire);
    let stack_top = DMESG_STACK_TOP.load(Ordering::Acquire);
    unsafe { enter_user(entry, stack_top, &args) }
}

pub fn spawn_kill_elf(args: Vec<String>) -> SpawnResult<u64> {
    spawn_elf(KILL_ELF, "kill", &KILL_ENTRY, &KILL_STACK_TOP, kill_entry, args)
}

extern "C" fn kill_entry() -> ! {
    x86_64::instructions::interrupts::enable();
    let id = crate::sched::current_id();
    let args = take_args_for(id);
    let entry = KILL_ENTRY.load(Ordering::Acquire);
    let stack_top = KILL_STACK_TOP.load(Ordering::Acquire);
    unsafe { enter_user(entry, stack_top, &args) }
}

pub fn spawn_uname_elf(args: Vec<String>) -> SpawnResult<u64> {
    spawn_elf(UNAME_ELF, "uname", &UNAME_ENTRY, &UNAME_STACK_TOP, uname_entry, args)
}

extern "C" fn uname_entry() -> ! {
    x86_64::instructions::interrupts::enable();
    let id = crate::sched::current_id();
    let args = take_args_for(id);
    let entry = UNAME_ENTRY.load(Ordering::Acquire);
    let stack_top = UNAME_STACK_TOP.load(Ordering::Acquire);
    unsafe { enter_user(entry, stack_top, &args) }
}

pub fn spawn_which_elf(args: Vec<String>) -> SpawnResult<u64> {
    spawn_elf(WHICH_ELF, "which", &WHICH_ENTRY, &WHICH_STACK_TOP, which_entry, args)
}

extern "C" fn which_entry() -> ! {
    x86_64::instructions::interrupts::enable();
    let id = crate::sched::current_id();
    let args = take_args_for(id);
    let entry = WHICH_ENTRY.load(Ordering::Acquire);
    let stack_top = WHICH_STACK_TOP.load(Ordering::Acquire);
    unsafe { enter_user(entry, stack_top, &args) }
}

pub fn spawn_env_elf(args: Vec<String>) -> SpawnResult<u64> {
    spawn_elf(ENV_ELF, "env", &ENV_ENTRY, &ENV_STACK_TOP, env_entry, args)
}

extern "C" fn env_entry() -> ! {
    x86_64::instructions::interrupts::enable();
    let id = crate::sched::current_id();
    let args = take_args_for(id);
    let entry = ENV_ENTRY.load(Ordering::Acquire);
    let stack_top = ENV_STACK_TOP.load(Ordering::Acquire);
    unsafe { enter_user(entry, stack_top, &args) }
}

pub fn spawn_viewer_elf(args: Vec<String>) -> SpawnResult<u64> {
    spawn_elf(VIEWER_ELF, "viewer", &VIEWER_ENTRY, &VIEWER_STACK_TOP, viewer_entry, args)
}

extern "C" fn viewer_entry() -> ! {
    x86_64::instructions::interrupts::enable();
    let id = crate::sched::current_id();
    let args = take_args_for(id);
    let entry = VIEWER_ENTRY.load(Ordering::Acquire);
    let stack_top = VIEWER_STACK_TOP.load(Ordering::Acquire);
    unsafe { enter_user(entry, stack_top, &args) }
}

pub fn spawn_play_elf(args: Vec<String>) -> SpawnResult<u64> {
    spawn_elf(PLAY_ELF, "play", &PLAY_ENTRY, &PLAY_STACK_TOP, play_entry, args)
}

extern "C" fn play_entry() -> ! {
    x86_64::instructions::interrupts::enable();
    let id = crate::sched::current_id();
    let args = take_args_for(id);
    let entry = PLAY_ENTRY.load(Ordering::Acquire);
    let stack_top = PLAY_STACK_TOP.load(Ordering::Acquire);
    unsafe { enter_user(entry, stack_top, &args) }
}

pub fn spawn_ping_elf(args: Vec<String>) -> SpawnResult<u64> {
    spawn_elf(PING_ELF, "ping", &PING_ENTRY, &PING_STACK_TOP, ping_entry, args)
}

extern "C" fn ping_entry() -> ! {
    x86_64::instructions::interrupts::enable();
    let id = crate::sched::current_id();
    let args = take_args_for(id);
    let entry = PING_ENTRY.load(Ordering::Acquire);
    let stack_top = PING_STACK_TOP.load(Ordering::Acquire);
    unsafe { enter_user(entry, stack_top, &args) }
}

pub fn exec_by_name(name: &str) -> SpawnResult<u64> {
    match name {
        "calc" | "calculator" => spawn_calculator_elf(),
        "worker" => spawn_worker_elf(),
        "shell" => spawn_shell_elf(),
        "echo" => spawn_echo_elf(Vec::new()),
        "ls" => spawn_ls_elf(Vec::new()),
        "cat" => spawn_cat_elf(Vec::new()),
        "ps" => spawn_ps_elf(),
        "cp" => spawn_cp_elf(Vec::new()),
        "wc" => spawn_wc_elf(Vec::new()),
        "head" => spawn_head_elf(Vec::new()),
        "dmesg" => spawn_dmesg_elf(Vec::new()),
        "kill" => spawn_kill_elf(Vec::new()),
        "uname" => spawn_uname_elf(Vec::new()),
        "which" => spawn_which_elf(Vec::new()),
        "env"   => spawn_env_elf(Vec::new()),
        "viewer" => spawn_viewer_elf(Vec::new()),
        "play" => spawn_play_elf(Vec::new()),
        "ping" => spawn_ping_elf(Vec::new()),
        _ => Err(SpawnError::UnknownProgram),
    }
}

pub fn exec_by_name_args(name: &str, args: Vec<String>) -> SpawnResult<u64> {
    match name {
        "calc" | "calculator" => spawn_calculator_elf(),
        "worker" => spawn_worker_elf(),
        "shell" => spawn_shell_elf(),
        "echo" => spawn_echo_elf(args),
        "ls" => spawn_ls_elf(args),
        "cat" => spawn_cat_elf(args),
        "ps" => spawn_ps_elf(),
        "cp" => spawn_cp_elf(args),
        "wc" => spawn_wc_elf(args),
        "head" => spawn_head_elf(args),
        "dmesg" => spawn_dmesg_elf(args),
        "kill" => spawn_kill_elf(args),
        "uname" => spawn_uname_elf(args),
        "which" => spawn_which_elf(args),
        "env"   => spawn_env_elf(args),
        "viewer" => spawn_viewer_elf(args),
        "play" => spawn_play_elf(args),
        "ping" => spawn_ping_elf(args),
        _ => Err(SpawnError::UnknownProgram),
    }
}

pub fn elf_bytes_by_name(name: &str) -> Option<&'static [u8]> {
    match name {
        "calc" | "calculator" => Some(CALC_ELF),
        "worker" => Some(WORKER_ELF),
        "shell" => Some(SHELL_ELF),
        "echo" => Some(ECHO_ELF),
        "ls" => Some(LS_ELF),
        "cat" => Some(CAT_ELF),
        "ps" => Some(PS_ELF),
        "cp" => Some(CP_ELF),
        "wc" => Some(WC_ELF),
        "head" => Some(HEAD_ELF),
        "dmesg" => Some(DMESG_ELF),
        "kill" => Some(KILL_ELF),
        "uname" => Some(UNAME_ELF),
        "which" => Some(WHICH_ELF),
        "env"   => Some(ENV_ELF),
        "viewer" => Some(VIEWER_ELF),
        "play" => Some(PLAY_ELF),
        "ping" => Some(PING_ELF),
        "user" => Some(USER_ELF),
        "init" => Some(INIT_ELF),
        _ => None,
    }
}

pub unsafe fn exec_in_place(
    elf: &[u8],
    args: Vec<String>,
    regs: *mut u64,
) -> SpawnResult<()> {
    let phys_offset_u64 = crate::memory::PHYS_OFFSET.load(Ordering::Acquire);
    if phys_offset_u64 == 0 {
        return Err(SpawnError::NoPhysOffset);
    }
    let phys_offset = VirtAddr::new(phys_offset_u64);

    let (old_frame, _old_flags) = Cr3::read();
    let old_pml4 = old_frame.start_address().as_u64();

    let loaded;
    let new_pml4;
    {
        let mut guard = crate::memory::FRAME_ALLOCATOR.lock();
        let alloc = guard.as_mut().ok_or(SpawnError::NoFrameAllocator)?;

        let new_as = crate::memory::AddressSpace::new_user(phys_offset, alloc)
            .ok_or(SpawnError::NoAddressSpace)?;
        new_pml4 = new_as.pml4_frame.start_address().as_u64();

        Cr3::write(new_as.pml4_frame, Cr3Flags::empty());

        let mut mapper = new_as.mapper(phys_offset);
        loaded = match crate::elf::load(elf, &mut mapper, alloc) {
            Ok(l) => l,
            Err(e) => {
                Cr3::write(old_frame, Cr3Flags::empty());
                crate::memory::AddressSpace::destroy(new_pml4);
                return Err(SpawnError::Elf(e));
            }
        };

        let stack_flags = PageTableFlags::PRESENT
            | PageTableFlags::WRITABLE
            | PageTableFlags::USER_ACCESSIBLE
            | PageTableFlags::NO_EXECUTE;
        if let Err(e) = map_user_region(
            &mut mapper, alloc,
            USER_STACK_BASE, USER_STACK_SIZE, stack_flags,
        ) {
            Cr3::write(old_frame, Cr3Flags::empty());
            crate::memory::AddressSpace::destroy(new_pml4);
            return Err(e);
        }
    }

    let (new_rsp, argv_ptr, argc) = setup_user_stack(USER_STACK_TOP_ADDR, &args);

    crate::sched::replace_current_pml4(new_pml4);
    crate::memory::AddressSpace::destroy(old_pml4);

    for i in 0..15 {
        *regs.add(i) = 0;
    }
    *regs.add(14) = argc;
    *regs.add(13) = argv_ptr;
    *regs.add(15) = loaded.entry;
    *regs.add(18) = new_rsp;

    Ok(())
}

pub unsafe fn exec_by_name_in_place(
    name: &str,
    args: Vec<String>,
    regs: *mut u64,
) -> SpawnResult<()> {
    let elf = elf_bytes_by_name(name).ok_or(SpawnError::UnknownProgram)?;
    exec_in_place(elf, args, regs)
}

pub unsafe fn fork_in_place(regs: *mut u64) -> SpawnResult<u64> {
    let phys_offset_u64 = crate::memory::PHYS_OFFSET.load(Ordering::Acquire);
    if phys_offset_u64 == 0 {
        return Err(SpawnError::NoPhysOffset);
    }
    let phys_offset = VirtAddr::new(phys_offset_u64);

    let parent_pml4 = crate::sched::current_pml4().ok_or(SpawnError::NoAddressSpace)?;

    let user_cs = *regs.add(16);
    let user_ss = *regs.add(19);
    let expected_cs = crate::gdt::user_code_selector().0 as u64;
    let expected_ss = crate::gdt::user_data_selector().0 as u64;
    if user_cs != expected_cs || user_ss != expected_ss {
        crate::log_warn!("[fork] parent not in Ring 3");
        return Err(SpawnError::UnknownProgram);
    }

    let child_pml4 = {
        let mut guard = crate::memory::FRAME_ALLOCATOR.lock();
        let alloc = guard.as_mut().ok_or(SpawnError::NoFrameAllocator)?;

        let child_as = crate::memory::AddressSpace::new_user(phys_offset, alloc)
            .ok_or(SpawnError::NoAddressSpace)?;
        let child_pml4 = child_as.pml4_frame.start_address().as_u64();

        if !crate::memory::clone_user_pages_cow(parent_pml4, child_pml4, phys_offset, alloc) {
            return Err(SpawnError::MapFailed);
        }
        child_pml4
    };

    let fr = ForkReturn {
        rip:    *regs.add(15),
        rflags: *regs.add(17),
        rsp:    *regs.add(18),
        rax:    0,
        rbx:    *regs.add(6),
        rbp:    *regs.add(5),
        r12:    *regs.add(4),
        r13:    *regs.add(3),
        r14:    *regs.add(2),
        r15:    *regs.add(1),
    };

    let parent_pid = crate::sched::current_id();

    let child_pid = x86_64::instructions::interrupts::without_interrupts(|| {
        let id = crate::sched::spawn_with_as("fork_child", fork_entry, child_pml4);
        PENDING_FORK_RETURNS.lock().insert(id, fr);
        id
    });

    crate::fd::fork_from(parent_pid, child_pid);
    Ok(child_pid)
}

extern "C" fn fork_entry() -> ! {
    x86_64::instructions::interrupts::enable();
    let pid = crate::sched::current_id();
    let fr = match PENDING_FORK_RETURNS.lock().remove(&pid) {
        Some(f) => f,
        None => {
            crate::log_error!("[fork] no pending return for pid {}", pid);
            crate::sched::exit_current_with_code(1);
        }
    };
    unsafe { iretq_fork(&fr) }
}

unsafe fn iretq_fork(fr: *const ForkReturn) -> ! {
    let user_cs = crate::gdt::user_code_selector().0 as u64;
    let user_ss = crate::gdt::user_data_selector().0 as u64;

    asm!(
        "mov rax, [rdx]",
        "mov rcx, [rdx + 8]",
        "mov r8,  [rdx + 16]",

        "push rdi",
        "push r8",
        "push rcx",
        "push rsi",
        "push rax",

        "mov rbx, [rdx + 32]",
        "mov rbp, [rdx + 40]",
        "mov r12, [rdx + 48]",
        "mov r13, [rdx + 56]",
        "mov r14, [rdx + 64]",
        "mov r15, [rdx + 72]",

        "mov rax, [rdx + 24]",
        "iretq",
        in("rdi") user_ss,
        in("rsi") user_cs,
        in("rdx") fr,
        options(noreturn)
    );
}

fn map_user_region(
    mapper: &mut impl Mapper<Size4KiB>,
    alloc: &mut impl FrameAllocator<Size4KiB>,
    base: u64,
    size: u64,
    flags: PageTableFlags,
) -> SpawnResult<()> {
    let start = Page::containing_address(VirtAddr::new(base));
    let end = Page::containing_address(VirtAddr::new(base + size - 1));
    for page in Page::range_inclusive(start, end) {
        let frame = alloc.allocate_frame().ok_or(SpawnError::MapFailed)?;
        unsafe {
            mapper.map_to(page, frame, flags, alloc)
                .map_err(|_| SpawnError::MapFailed)?
                .flush();
        }
    }
    Ok(())
}

unsafe fn setup_user_stack(stack_top: u64, args: &[String]) -> (u64, u64, u64) {
    let mut sp = stack_top;
    let mut str_ptrs: Vec<u64> = Vec::new();
    for arg in args.iter().rev() {
        let bytes = arg.as_bytes();
        let total = bytes.len() as u64 + 1;
        sp -= total;
        let dst = sp as *mut u8;
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), dst, bytes.len());
        *dst.add(bytes.len()) = 0;
        str_ptrs.push(sp);
        sp &= !7;
    }
    str_ptrs.reverse();

    sp -= 8;
    *(sp as *mut u64) = 0;
    for p in str_ptrs.iter().rev() {
        sp -= 8;
        *(sp as *mut u64) = *p;
    }
    let argv_ptr = sp;

    sp -= 8;
    *(sp as *mut u64) = args.len() as u64;

    sp &= !15;
    sp -= 8;

    (sp, argv_ptr, args.len() as u64)
}

pub unsafe fn enter_user(entry: u64, stack_top: u64, args: &[String]) -> ! {
    let user_cs = crate::gdt::user_code_selector().0 as u64;
    let user_ss = crate::gdt::user_data_selector().0 as u64;

    let (new_rsp, argv_ptr, argc) = setup_user_stack(stack_top, args);

    asm!(
        "push rax",
        "push rcx",
        "push rdx",
        "push rsi",
        "push rdi",
        "mov rdi, r8",
        "mov rsi, r9",
        "iretq",
        in("rax") user_ss,
        in("rcx") new_rsp,
        in("rdx") 0x202u64,
        in("rsi") user_cs,
        in("rdi") entry,
        in("r8") argc,
        in("r9") argv_ptr,
        options(noreturn)
    );
}