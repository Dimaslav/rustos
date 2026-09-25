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

use crate::{println, serial_println};

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
        serial_println!(
            "[pic] master mask = {:#010b}, slave mask = {:#010b}",
            m, s
        );
        if m & 0b100 != 0 {
            serial_println!("[pic] !!! IRQ2 (cascade) замаскирован !!!");
        }
        if s & 0b0001_0000 != 0 {
            serial_println!("[pic] !!! IRQ12 (mouse) замаскирован !!!");
        }

        crate::mouse::init();
    }

    x86_64::instructions::interrupts::enable();
}

extern "x86-interrupt" fn breakpoint_handler(sf: InterruptStackFrame) {
    serial_println!("EXCEPTION: BREAKPOINT");
    serial_println!("{:#?}", sf);
    println!("EXCEPTION: BREAKPOINT");
}

extern "x86-interrupt" fn double_fault_handler(
    sf: InterruptStackFrame,
    _error_code: u64,
) -> ! {
    serial_println!("EXCEPTION: DOUBLE FAULT");
    serial_println!("{:#?}", sf);
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

    serial_println!("EXCEPTION: PAGE FAULT");
    serial_println!("  addr:       {:?}", addr);
    serial_println!("  protection: {}", protection);
    serial_println!("  write:      {}", write);
    serial_println!("  user:       {}", user);
    serial_println!("  instr:      {}", instr);
    serial_println!("{:#?}", sf);

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
    serial_println!("EXCEPTION: GENERAL PROTECTION FAULT (code={})", error_code);
    serial_println!("{:#?}", sf);
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