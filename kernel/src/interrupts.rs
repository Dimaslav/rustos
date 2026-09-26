use core::arch::asm;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use pic8259::ChainedPics;
use spin::{Mutex, Once};
use x86_64::instructions::port::Port;
use x86_64::registers::control::Cr2;
use x86_64::structures::idt::{
    InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode,
};
use x86_64::{PrivilegeLevel, VirtAddr};

use crate::println;

pub const PIC_1_OFFSET: u8 = 32;
pub const PIC_2_OFFSET: u8 = PIC_1_OFFSET + 8;
pub const MOUSE_IRQ: u8 = PIC_2_OFFSET + 4;

pub static PICS: Mutex<ChainedPics> =
    Mutex::new(unsafe { ChainedPics::new(PIC_1_OFFSET, PIC_2_OFFSET) });

pub static TICKS: AtomicU64 = AtomicU64::new(0);
static IDT: Once<InterruptDescriptorTable> = Once::new();

static CLOCK_DIRTY: AtomicBool = AtomicBool::new(false);

pub fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

pub const TIMER_HZ: u64 = 18;

pub fn uptime_secs() -> u64 {
    ticks() / TIMER_HZ
}

pub fn clock_tick() {
    CLOCK_DIRTY.store(true, Ordering::Release);
}

pub fn take_clock_dirty() -> bool {
    CLOCK_DIRTY.swap(false, Ordering::AcqRel)
}

fn read_status() -> u8 {
    unsafe { Port::<u8>::new(0x64).read() }
}

fn read_data() -> u8 {
    unsafe { Port::<u8>::new(0x60).read() }
}

pub fn init() {
    let mut idt = InterruptDescriptorTable::new();
    idt.breakpoint.set_handler_fn(breakpoint_handler);
    unsafe {
        idt.double_fault
            .set_handler_fn(double_fault_handler)
            .set_stack_index(crate::gdt::DOUBLE_FAULT_IST_INDEX);
    }
    idt.page_fault.set_handler_fn(page_fault_handler);
    idt.general_protection_fault.set_handler_fn(gpf_handler);
    idt[PIC_1_OFFSET].set_handler_fn(timer_handler);
    idt[PIC_1_OFFSET + 1].set_handler_fn(keyboard_handler);
    idt[MOUSE_IRQ].set_handler_fn(mouse_handler);

    // int 0x80 — syscall. Доступен из Ring 3.
    unsafe {
        idt[0x80]
            .set_handler_addr(VirtAddr::new(
                crate::syscall::syscall_entry as *const () as u64,
            ))
            .set_privilege_level(PrivilegeLevel::Ring3);
    }

    IDT.call_once(|| idt).load();

    unsafe {
        PICS.lock().initialize();
        let mut master_mask: Port<u8> = Port::new(0x21);
        let mut slave_mask: Port<u8> = Port::new(0xA1);
        master_mask.write(0b1111_1000); // IRQ0, IRQ1, IRQ2 (cascade)
        slave_mask.write(0b1110_1111);  // IRQ12

        let m = Port::<u8>::new(0x21).read();
        let s = Port::<u8>::new(0xA1).read();
        crate::log_info!(
            "[pic] master mask = {:#010b}, slave mask = {:#010b}",
            m, s
        );
        if m & 0b100 != 0 {
            crate::log_error!("[pic] IRQ2 (cascade) замаскирован — PS/2 мышь не будет работать");
        }
        if s & 0b0001_0000 != 0 {
            crate::log_error!("[pic] IRQ12 (mouse) замаскирован — PS/2 мышь не будет работать");
        }

        crate::mouse::init();
    }

    x86_64::instructions::interrupts::enable();
}

extern "x86-interrupt" fn breakpoint_handler(sf: InterruptStackFrame) {
    crate::log_warn!("EXCEPTION: BREAKPOINT");
    crate::log_warn!("{:#?}", sf);
    println!("EXCEPTION: BREAKPOINT");
}

extern "x86-interrupt" fn double_fault_handler(
    sf: InterruptStackFrame,
    _error_code: u64,
) -> ! {
    crate::log_error!("EXCEPTION: DOUBLE FAULT");
    crate::log_error!("{:#?}", sf);
    println!("EXCEPTION: DOUBLE FAULT");
    loop {
        unsafe { asm!("hlt"); }
    }
}

extern "x86-interrupt" fn page_fault_handler(
    sf: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    let addr = Cr2::read();
    let protection = error_code.contains(PageFaultErrorCode::PROTECTION_VIOLATION);
    let write = error_code.contains(PageFaultErrorCode::CAUSED_BY_WRITE);
    let user = error_code.contains(PageFaultErrorCode::USER_MODE);
    let instr = error_code.contains(PageFaultErrorCode::INSTRUCTION_FETCH);

    crate::log_error!("EXCEPTION: PAGE FAULT");
    crate::log_error!("  addr:       {:?}", addr);
    crate::log_error!("  protection: {}", protection);
    crate::log_error!("  write:      {}", write);
    crate::log_error!("  user:       {}", user);
    crate::log_error!("  instr:      {}", instr);
    crate::log_error!("  rip:        {:?}", sf.instruction_pointer);

    if user {
        // User-поток упал. Убиваем его — kernel продолжает работу.
        // Это изоляция: падение user-программы не вешает GUI.
        crate::log_warn!("[fault] killing user thread");

        // Восстанавливаем нормальный kernel CR3 (на случай, если fault
        // произошёл в user-AS, и мы сейчас на user-таблицах).
        // Scheduler сам подставит нужный CR3 при следующем switch.

        crate::sched::exit_current();
    }

    // Kernel fault — аварийная остановка.
    crate::log_error!("{:#?}", sf);
    println!(
        "EXCEPTION: PAGE FAULT addr={:?} w={} p={}",
        addr, write, protection
    );
    loop {
        unsafe { asm!("hlt"); }
    }
}

extern "x86-interrupt" fn gpf_handler(
    sf: InterruptStackFrame,
    error_code: u64,
) {
    crate::log_error!("EXCEPTION: GENERAL PROTECTION FAULT (code={})", error_code);
    crate::log_error!("{:#?}", sf);
    println!(
        "EXCEPTION: GENERAL PROTECTION FAULT (code={})",
        error_code
    );
    loop {
        unsafe { asm!("hlt"); }
    }
}

extern "x86-interrupt" fn timer_handler(_sf: InterruptStackFrame) {
    TICKS.fetch_add(1, Ordering::Relaxed);
    crate::task::sleep::wake_expired();

    // EOI до context switch: PIC не должен ждать, пока мы вернёмся.
    unsafe {
        PICS.lock().notify_end_of_interrupt(PIC_1_OFFSET);
    }

    crate::sched::schedule_tick();
}

extern "x86-interrupt" fn keyboard_handler(_sf: InterruptStackFrame) {
    for _ in 0..32 {
        let status = read_status();
        if status & 0x01 == 0 {
            break;
        }
        let b = read_data();
        if status & 0x20 != 0 {
            crate::mouse::push_byte_irq(b);
        } else {
            crate::keyboard::handle(b);
        }
    }
    unsafe {
        PICS.lock().notify_end_of_interrupt(PIC_1_OFFSET + 1);
    }
}

extern "x86-interrupt" fn mouse_handler(_sf: InterruptStackFrame) {
    crate::mouse::IRQ_STARTS.fetch_add(1, Ordering::Relaxed);

    for _ in 0..32 {
        let status = read_status();
        if status & 0x01 == 0 {
            break;
        }
        let b = read_data();
        if status & 0x20 != 0 {
            crate::mouse::push_byte_irq(b);
        } else {
            crate::keyboard::handle(b);
        }
    }
    unsafe {
        PICS.lock().notify_end_of_interrupt(MOUSE_IRQ);
    }
}