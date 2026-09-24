use core::arch::asm;
use core::sync::atomic::{AtomicU64, Ordering};

use pic8259::ChainedPics;
use spin::{Mutex, Once};
use x86_64::instructions::port::Port;
use x86_64::structures::idt::{
    InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode,
};

use crate::println;

pub const PIC_1_OFFSET: u8 = 32;
pub const PIC_2_OFFSET: u8 = PIC_1_OFFSET + 8;
pub const MOUSE_IRQ: u8 = PIC_2_OFFSET + 4; // IRQ12

pub static PICS: Mutex<ChainedPics> =
    Mutex::new(unsafe { ChainedPics::new(PIC_1_OFFSET, PIC_2_OFFSET) });

pub static TICKS: AtomicU64 = AtomicU64::new(0);

static IDT: Once<InterruptDescriptorTable> = Once::new();

pub fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

/// Секунды с момента загрузки (PIT ≈ 18.22 Гц).
pub fn uptime_secs() -> u64 {
    ticks() * 100 / 1822
}

pub fn init() {
    let mut idt = InterruptDescriptorTable::new();
    idt.breakpoint.set_handler_fn(breakpoint_handler);
    idt.double_fault.set_handler_fn(double_fault_handler);
    idt.page_fault.set_handler_fn(page_fault_handler);
    idt.general_protection_fault.set_handler_fn(gpf_handler);
    idt[PIC_1_OFFSET].set_handler_fn(timer_handler);
    idt[PIC_1_OFFSET + 1].set_handler_fn(keyboard_handler);
    idt[MOUSE_IRQ].set_handler_fn(mouse_handler);

    IDT.call_once(|| idt).load();

    unsafe {
        PICS.lock().initialize();

        // Маскируем всё, кроме IRQ0 (таймер) и IRQ1 (клавиатура) на master,
        // и IRQ12 (мышь) на slave.
        let mut master_mask: Port<u8> = Port::new(0x21);
        let mut slave_mask: Port<u8> = Port::new(0xA1);
        master_mask.write(0b1111_1000); // IRQ0, IRQ1, IRQ2 (cascade) открыты
        slave_mask.write(0b1110_1111);  // IRQ12 открыт (bit 4)

        crate::mouse::init();
    }

    x86_64::instructions::interrupts::enable();
}

extern "x86-interrupt" fn breakpoint_handler(sf: InterruptStackFrame) {
    println!("EXCEPTION: BREAKPOINT\n{:#?}", sf);
}

extern "x86-interrupt" fn double_fault_handler(sf: InterruptStackFrame, _e: u64) -> ! {
    println!("EXCEPTION: DOUBLE FAULT\n{:#?}", sf);
    loop {
        unsafe { asm!("hlt"); }
    }
}

extern "x86-interrupt" fn page_fault_handler(sf: InterruptStackFrame, _e: PageFaultErrorCode) {
    println!("EXCEPTION: PAGE FAULT\n{:#?}", sf);
    loop {
        unsafe { asm!("hlt"); }
    }
}

extern "x86-interrupt" fn gpf_handler(sf: InterruptStackFrame, _e: u64) {
    println!("EXCEPTION: GENERAL PROTECTION FAULT\n{:#?}", sf);
    loop {
        unsafe { asm!("hlt"); }
    }
}

extern "x86-interrupt" fn timer_handler(_sf: InterruptStackFrame) {
    TICKS.fetch_add(1, Ordering::Relaxed);
    unsafe {
        PICS.lock().notify_end_of_interrupt(PIC_1_OFFSET);
    }
}

extern "x86-interrupt" fn keyboard_handler(_sf: InterruptStackFrame) {
    let mut port: Port<u8> = Port::new(0x60);
    let scancode: u8 = unsafe { port.read() };
    crate::keyboard::handle(scancode);
    unsafe {
        PICS.lock().notify_end_of_interrupt(PIC_1_OFFSET + 1);
    }
}

extern "x86-interrupt" fn mouse_handler(_sf: InterruptStackFrame) {
    let mut port: Port<u8> = Port::new(0x60);
    let byte: u8 = unsafe { port.read() };
    crate::mouse::push_byte(byte);
    unsafe {
        PICS.lock().notify_end_of_interrupt(MOUSE_IRQ);
    }
}