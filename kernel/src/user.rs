//! Запуск Ring 3: отдельное адресное пространство, загрузка ELF, iretq.

use core::arch::asm;
use core::sync::atomic::{AtomicU64, Ordering};

use x86_64::{
    registers::control::{Cr3, Cr3Flags},
    structures::paging::{
        FrameAllocator, Mapper, Page, PageTableFlags, Size4KiB,
    },
    VirtAddr,
};

use crate::{log_debug, log_info};

const USER_STACK_BASE: u64 = 0x80_0000;
const USER_STACK_SIZE: u64 = 16 * 1024;

static USER_ELF: &[u8] = include_bytes!("../user.elf");
static WORKER_ELF: &[u8] = include_bytes!("../worker.elf");
static CALC_ELF: &[u8] = include_bytes!("../calculator.elf");
static SHELL_ELF: &[u8] = include_bytes!("../shell.elf");

static USER_ENTRY: AtomicU64 = AtomicU64::new(0);
static USER_STACK_TOP: AtomicU64 = AtomicU64::new(0);
static WORKER_ENTRY: AtomicU64 = AtomicU64::new(0);
static WORKER_STACK_TOP: AtomicU64 = AtomicU64::new(0);
static CALC_ENTRY: AtomicU64 = AtomicU64::new(0);
static CALC_STACK_TOP: AtomicU64 = AtomicU64::new(0);
static SHELL_ENTRY: AtomicU64 = AtomicU64::new(0);
static SHELL_STACK_TOP: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub enum SpawnError {
    /// `PHYS_OFFSET` не инициализирован (kernel_main не дошёл до memory::init).
    NoPhysOffset,
    /// `FRAME_ALLOCATOR` не инициализирован.
    NoFrameAllocator,
    /// Не удалось выделить PML4 под user-AS.
    NoAddressSpace,
    /// ELF не загрузился — см. `ElfError`.
    Elf(crate::elf::ElfError),
    /// Не удалось замапить страницы стека/сегментов.
    MapFailed,
    /// Запрошена незнакомая программа для `exec`.
    UnknownProgram,
}

pub type SpawnResult<T> = Result<T, SpawnError>;

fn spawn_elf(
    elf: &[u8],
    name: &'static str,
    entry_slot: &AtomicU64,
    stack_slot: &AtomicU64,
    entry_fn: extern "C" fn() -> !,
) -> SpawnResult<u64> {
    let phys_offset_u64 =
        crate::memory::PHYS_OFFSET.load(Ordering::Acquire);
    if phys_offset_u64 == 0 {
        return Err(SpawnError::NoPhysOffset);
    }
    let phys_offset = VirtAddr::new(phys_offset_u64);

    let mut guard = crate::memory::FRAME_ALLOCATOR.lock();
    let alloc = guard.as_mut().ok_or(SpawnError::NoFrameAllocator)?;
    let as_ = unsafe {
        crate::memory::AddressSpace::new_user(phys_offset, alloc)
            .ok_or(SpawnError::NoAddressSpace)?
    };

    // Загрузка ELF во временно активном user-AS.
    // Прерывания запрещены, чтобы таймер не сработал с чужим CR3.
    let loaded = unsafe {
        x86_64::instructions::interrupts::without_interrupts(
            || -> SpawnResult<crate::elf::LoadedElf> {
                let (saved_frame, saved_flags) = Cr3::read();
                Cr3::write(as_.pml4_frame, Cr3Flags::empty());

                // Все пути выхода должны восстановить CR3 — поэтому
                // используем ручной match, а не `?`.
                let mut mapper = as_.mapper(phys_offset);

                let loaded = match crate::elf::load(elf, &mut mapper, alloc) {
                    Ok(l) => l,
                    Err(e) => {
                        Cr3::write(saved_frame, saved_flags);
                        return Err(SpawnError::Elf(e));
                    }
                };

                // Диагностика: читаем байты по entry в user-AS.
                let p = loaded.entry as *const u8;
                let mut buf = [0u8; 16];
                for i in 0..16 {
                    buf[i] = core::ptr::read_volatile(p.add(i));
                }
                log_debug!("[elf] bytes @ entry({:#x}): {:02x?}", loaded.entry, buf);

                let map_res = map_user_region(
                    &mut mapper,
                    alloc,
                    USER_STACK_BASE,
                    USER_STACK_SIZE,
                    PageTableFlags::PRESENT
                        | PageTableFlags::WRITABLE
                        | PageTableFlags::USER_ACCESSIBLE,
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
    Ok(crate::sched::spawn_with_as(name, entry_fn, pml4_phys))
}

/// Загрузка user-программы. Ранее паниковала — теперь возвращает `Result`.
///
/// # Safety
/// Вызывается до первого входа в Ring 3.
pub unsafe fn spawn_user_test(_phys_offset: VirtAddr) -> SpawnResult<u64> {
    let id = spawn_elf(USER_ELF, "user", &USER_ENTRY, &USER_STACK_TOP, user_entry)?;
    log_info!(
        "[user] ELF loaded: entry={:#x}",
        USER_ENTRY.load(Ordering::Acquire)
    );
    Ok(id)
}

extern "C" fn user_entry() -> ! {
    x86_64::instructions::interrupts::enable();
    let entry = USER_ENTRY.load(Ordering::Acquire);
    let stack_top = USER_STACK_TOP.load(Ordering::Acquire);
    let cs = crate::gdt::user_code_selector().0 as u64;
    let ss = crate::gdt::user_data_selector().0 as u64;

    log_debug!(
        "[user_entry] entry={:#x} stack={:#x} cs={:#x} ss={:#x}",
        entry, stack_top, cs, ss
    );

    unsafe { enter_user(entry, stack_top) }
}

pub fn spawn_worker_elf() -> SpawnResult<u64> {
    spawn_elf(WORKER_ELF, "worker", &WORKER_ENTRY, &WORKER_STACK_TOP, worker_entry)
}

extern "C" fn worker_entry() -> ! {
    x86_64::instructions::interrupts::enable();
    let entry = WORKER_ENTRY.load(Ordering::Acquire);
    let stack_top = WORKER_STACK_TOP.load(Ordering::Acquire);
    let cs = crate::gdt::user_code_selector().0 as u64;
    let ss = crate::gdt::user_data_selector().0 as u64;
    log_debug!(
        "[worker_entry] entry={:#x} stack={:#x} cs={:#x} ss={:#x}",
        entry, stack_top, cs, ss
    );
    unsafe { enter_user(entry, stack_top) }
}

pub fn spawn_calculator_elf() -> SpawnResult<u64> {
    spawn_elf(CALC_ELF, "calc", &CALC_ENTRY, &CALC_STACK_TOP, calc_entry)
}

extern "C" fn calc_entry() -> ! {
    x86_64::instructions::interrupts::enable();
    let entry = CALC_ENTRY.load(Ordering::Acquire);
    let stack_top = CALC_STACK_TOP.load(Ordering::Acquire);
    let cs = crate::gdt::user_code_selector().0 as u64;
    let ss = crate::gdt::user_data_selector().0 as u64;
    log_debug!(
        "[calc_entry] entry={:#x} stack={:#x} cs={:#x} ss={:#x}",
        entry, stack_top, cs, ss
    );
    unsafe { enter_user(entry, stack_top) }
}

pub fn spawn_shell_elf() -> SpawnResult<u64> {
    spawn_elf(SHELL_ELF, "shell", &SHELL_ENTRY, &SHELL_STACK_TOP, shell_entry)
}

extern "C" fn shell_entry() -> ! {
    x86_64::instructions::interrupts::enable();
    let entry = SHELL_ENTRY.load(Ordering::Acquire);
    let stack_top = SHELL_STACK_TOP.load(Ordering::Acquire);
    let cs = crate::gdt::user_code_selector().0 as u64;
    let ss = crate::gdt::user_data_selector().0 as u64;
    log_debug!(
        "[shell_entry] entry={:#x} stack={:#x} cs={:#x} ss={:#x}",
        entry, stack_top, cs, ss
    );
    unsafe { enter_user(entry, stack_top) }
}

pub fn exec_by_name(name: &str) -> SpawnResult<u64> {
    match name {
        "calc" | "calculator" => spawn_calculator_elf(),
        "worker" => spawn_worker_elf(),
        "shell" => spawn_shell_elf(),
        _ => Err(SpawnError::UnknownProgram),
    }
}

/// Замапить диапазон user-страниц с одинаковыми флагами.
/// Возвращает `SpawnError::MapFailed` при нехватке фреймов или ошибке маппинга.
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
            mapper
                .map_to(page, frame, flags, alloc)
                .map_err(|_| SpawnError::MapFailed)?
                .flush();
        }
    }
    Ok(())
}

/// Переход Ring 0 → Ring 3.
///
/// Каждый входной операнд жёстко закреплён за регистром (`in("rax")`,
/// `in("rdi")`, ...), затем просто пушится на стек — компилятор не может
/// перераспределить регистры.
///
/// Порядок iretq-фрейма: [RIP][CS][RFLAGS][RSP][SS].
/// Пушим снизу вверх, значит последним push'им RIP.
///
/// # Safety
/// Валидные селекторы, entry и stack_top; CR3 = user-AS.
pub unsafe fn enter_user(entry: u64, stack_top: u64) -> ! {
    let user_cs = crate::gdt::user_code_selector().0 as u64;
    let user_ss = crate::gdt::user_data_selector().0 as u64;

    asm!(
        "push rax",
        "push rcx",
        "push rdx",
        "push rsi",
        "push rdi",
        "iretq",
        in("rax") user_ss,
        in("rcx") stack_top,
        in("rdx") 0x202u64,
        in("rsi") user_cs,
        in("rdi") entry,
        options(noreturn)
    );
}