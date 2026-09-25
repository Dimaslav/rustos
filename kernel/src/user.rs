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

const USER_STACK_BASE: u64 = 0x80_0000;
const USER_STACK_SIZE: u64 = 16 * 1024;

static USER_ELF: &[u8] = include_bytes!("../user.elf");
static WORKER_ELF: &[u8] = include_bytes!("../worker.elf");
static CALC_ELF: &[u8] = include_bytes!("../calculator.elf");

static USER_ENTRY: AtomicU64 = AtomicU64::new(0);
static USER_STACK_TOP: AtomicU64 = AtomicU64::new(0);
static WORKER_ENTRY: AtomicU64 = AtomicU64::new(0);
static WORKER_STACK_TOP: AtomicU64 = AtomicU64::new(0);
static CALC_ENTRY: AtomicU64 = AtomicU64::new(0);
static CALC_STACK_TOP: AtomicU64 = AtomicU64::new(0);

pub unsafe fn spawn_user_test(phys_offset: VirtAddr) {
    let mut guard = crate::memory::FRAME_ALLOCATOR.lock();
    let alloc = guard.as_mut().expect("frame allocator not initialized");
    let as_ = crate::memory::AddressSpace::new_user(phys_offset, alloc)
        .expect("cannot create address space");

    let loaded = x86_64::instructions::interrupts::without_interrupts(|| {
        let (saved_frame, saved_flags) = Cr3::read();
        Cr3::write(as_.pml4_frame, Cr3Flags::empty());
        let mut mapper = as_.mapper(phys_offset);
        let loaded = crate::elf::load(USER_ELF, &mut mapper, alloc).expect("ELF load failed");
        map_user_region(&mut mapper, alloc, USER_STACK_BASE, USER_STACK_SIZE,
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE);
        Cr3::write(saved_frame, saved_flags);
        loaded
    });

    let pml4_phys = as_.pml4_frame.start_address().as_u64();
    drop(guard);

    crate::serial_println!("[user] ELF loaded: entry={:#x}, segments={}", loaded.entry, loaded.segments.len());
    USER_ENTRY.store(loaded.entry, Ordering::Release);
    USER_STACK_TOP.store(USER_STACK_BASE + USER_STACK_SIZE, Ordering::Release);
    crate::sched::spawn_with_as("user", user_entry, pml4_phys);
}

extern "C" fn user_entry() -> ! {
    x86_64::instructions::interrupts::enable();
    let entry = USER_ENTRY.load(Ordering::Acquire);
    let stack_top = USER_STACK_TOP.load(Ordering::Acquire);
    unsafe { enter_user(entry, stack_top) }
}

pub fn spawn_worker_elf() -> Option<u64> {
    let phys_offset = VirtAddr::new(crate::memory::PHYS_OFFSET.load(Ordering::Acquire));
    let mut guard = crate::memory::FRAME_ALLOCATOR.lock();
    let alloc = guard.as_mut()?;
    let as_ = unsafe { crate::memory::AddressSpace::new_user(phys_offset, alloc)? };
    let loaded = unsafe {
        x86_64::instructions::interrupts::without_interrupts(|| {
            let (saved_frame, saved_flags) = Cr3::read();
            Cr3::write(as_.pml4_frame, Cr3Flags::empty());
            let mut mapper = as_.mapper(phys_offset);
            let loaded = crate::elf::load(WORKER_ELF, &mut mapper, alloc).expect("worker ELF load failed");
            map_user_region(&mut mapper, alloc, USER_STACK_BASE, USER_STACK_SIZE,
                PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE);
            Cr3::write(saved_frame, saved_flags);
            loaded
        })
    };
    let pml4_phys = as_.pml4_frame.start_address().as_u64();
    drop(guard);
    WORKER_ENTRY.store(loaded.entry, Ordering::Release);
    WORKER_STACK_TOP.store(USER_STACK_BASE + USER_STACK_SIZE, Ordering::Release);
    Some(crate::sched::spawn_with_as("worker", worker_entry, pml4_phys))
}

extern "C" fn worker_entry() -> ! {
    x86_64::instructions::interrupts::enable();
    let entry = WORKER_ENTRY.load(Ordering::Acquire);
    let stack_top = WORKER_STACK_TOP.load(Ordering::Acquire);
    unsafe { enter_user(entry, stack_top) }
}

pub fn spawn_calculator_elf() -> Option<u64> {
    let phys_offset = VirtAddr::new(crate::memory::PHYS_OFFSET.load(Ordering::Acquire));
    let mut guard = crate::memory::FRAME_ALLOCATOR.lock();
    let alloc = guard.as_mut()?;
    let as_ = unsafe { crate::memory::AddressSpace::new_user(phys_offset, alloc)? };
    let loaded = unsafe {
        x86_64::instructions::interrupts::without_interrupts(|| {
            let (saved_frame, saved_flags) = Cr3::read();
            Cr3::write(as_.pml4_frame, Cr3Flags::empty());
            let mut mapper = as_.mapper(phys_offset);
            let loaded = crate::elf::load(CALC_ELF, &mut mapper, alloc).expect("calc ELF load failed");
            map_user_region(&mut mapper, alloc, USER_STACK_BASE, USER_STACK_SIZE,
                PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE);
            Cr3::write(saved_frame, saved_flags);
            loaded
        })
    };
    let pml4_phys = as_.pml4_frame.start_address().as_u64();
    drop(guard);
    CALC_ENTRY.store(loaded.entry, Ordering::Release);
    CALC_STACK_TOP.store(USER_STACK_BASE + USER_STACK_SIZE, Ordering::Release);
    Some(crate::sched::spawn_with_as("calc", calc_entry, pml4_phys))
}

extern "C" fn calc_entry() -> ! {
    x86_64::instructions::interrupts::enable();
    let entry = CALC_ENTRY.load(Ordering::Acquire);
    let stack_top = CALC_STACK_TOP.load(Ordering::Acquire);
    unsafe { enter_user(entry, stack_top) }
}

fn map_user_region(
    mapper: &mut impl Mapper<Size4KiB>,
    alloc: &mut impl FrameAllocator<Size4KiB>,
    base: u64, size: u64, flags: PageTableFlags,
) {
    let start = Page::containing_address(VirtAddr::new(base));
    let end = Page::containing_address(VirtAddr::new(base + size - 1));
    for page in Page::range_inclusive(start, end) {
        let frame = alloc.allocate_frame().expect("no frame for user");
        unsafe {
            mapper.map_to(page, frame, flags, alloc).expect("map_to user failed").flush();
        }
    }
}

pub unsafe fn enter_user(entry: u64, stack_top: u64) -> ! {
    let user_cs = crate::gdt::user_code_selector().0 as u64;
    let user_ss = crate::gdt::user_data_selector().0 as u64;
    let user_ss_16 = crate::gdt::user_data_selector().0;

    asm!(
        "mov ax, {ss16:x}",
        "mov ds, ax",
        "mov es, ax",
        "push {ss}",
        "push {rsp}",
        "push {flags}",
        "push {cs}",
        "push {rip}",
        "iretq",
        ss16 = in(reg) user_ss_16,
        ss = in(reg) user_ss,
        rsp = in(reg) stack_top,
        flags = in(reg) 0x202u64,
        cs = in(reg) user_cs,
        rip = in(reg) entry,
        options(noreturn)
    );
}