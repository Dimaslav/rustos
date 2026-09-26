//! Запуск Ring 3: отдельное адресное пространство, загрузка ELF, iretq.

use alloc::string::String;
use alloc::vec::Vec;
use core::arch::asm;
use core::sync::atomic::{AtomicU64, Ordering};

use x86_64::{
    registers::control::{Cr3, Cr3Flags},
    structures::paging::{FrameAllocator, Mapper, Page, PageTableFlags, Size4KiB},
    VirtAddr,
};

use crate::{log_debug, log_info};

const USER_STACK_BASE: u64 = 0x80_0000;
const USER_STACK_SIZE: u64 = 32 * 1024;

static USER_ELF: &[u8] = include_bytes!("../user.elf");
static WORKER_ELF: &[u8] = include_bytes!("../worker.elf");
static CALC_ELF: &[u8] = include_bytes!("../calculator.elf");
static SHELL_ELF: &[u8] = include_bytes!("../shell.elf");
static ECHO_ELF: &[u8] = include_bytes!("../echo.elf");
static LS_ELF: &[u8] = include_bytes!("../ls.elf");
static CAT_ELF: &[u8] = include_bytes!("../cat.elf");
static PS_ELF: &[u8] = include_bytes!("../ps.elf");

static USER_ENTRY: AtomicU64 = AtomicU64::new(0);
static USER_STACK_TOP: AtomicU64 = AtomicU64::new(0);
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

static PENDING_ARGS: spin::Mutex<Vec<(u64, Vec<String>)>> = spin::Mutex::new(Vec::new());

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

                let map_res = map_user_region(
                    &mut mapper, alloc,
                    USER_STACK_BASE, USER_STACK_SIZE,
                    PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE,
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
    stack_slot.store(USER_STACK_BASE + USER_STACK_SIZE, Ordering::Release);

    let parent = crate::sched::current_id();
    let id = crate::sched::spawn_with_as(name, entry_fn, pml4_phys);
    crate::fd::fork_from(parent, id);
    PENDING_ARGS.lock().push((id, args));
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

pub fn exec_by_name(name: &str) -> SpawnResult<u64> {
    match name {
        "calc" | "calculator" => spawn_calculator_elf(),
        "worker" => spawn_worker_elf(),
        "shell" => spawn_shell_elf(),
        "echo" => spawn_echo_elf(Vec::new()),
        "ls" => spawn_ls_elf(Vec::new()),
        "cat" => spawn_cat_elf(Vec::new()),
        "ps" => spawn_ps_elf(),
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
        _ => Err(SpawnError::UnknownProgram),
    }
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

    (sp, argv_ptr, args.len() as u64)
}

/// Переход Ring 0 → Ring 3 с argv (rdi=argc, rsi=argv).
///
/// # Safety
/// Валидные селекторы, entry, stack_top; CR3 = user-AS.
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