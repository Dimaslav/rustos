use core::arch::asm;

use pic8259::ChainedPics;
use spin::{Mutex, Once};
use x86_64::instructions::port::Port;
use x86_64::structures::idt::{
    InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode,
};

use crate::keyboard::{scancode_to_ascii, KEY_QUEUE};
use crate::println;

pub const PIC_1_OFFSET: u8 = 32;
pub const PIC_2_OFFSET: u8 = PIC_1_OFFSET + 8;

pub static PICS: Mutex<ChainedPics> =
    Mutex::new(unsafe { ChainedPics::new(PIC_1_OFFSET, PIC_2_OFFSET) });

static IDT: Once<InterruptDescriptorTable> = Once::new();

pub fn init() {
    let mut idt = InterruptDescriptorTable::new();
    idt.breakpoint.set_handler_fn(breakpoint_handler);
    idt.double_fault.set_handler_fn(double_fault_handler);
    idt.page_fault.set_handler_fn(page_fault_handler);
    idt.general_protection_fault.set_handler_fn(gpf_handler);
    idt[PIC_1_OFFSET].set_handler_fn(timer_handler);
    idt[PIC_1_OFFSET + 1].set_handler_fn(keyboard_handler);

    IDT.call_once(|| idt).load();

    unsafe {
        PICS.lock().initialize();
    }

    x86_64::instructions::interrupts::enable();
}

extern "x86-interrupt" fn breakpoint_handler(stack_frame: InterruptStackFrame) {
    println!("EXCEPTION: BREAKPOINT\n{:#?}", stack_frame);
}

extern "x86-interrupt" fn double_fault_handler(
    stack_frame: InterruptStackFrame,
    _error_code: u64,
) -> ! {
    println!("EXCEPTION: DOUBLE FAULT\n{:#?}", stack_frame);
    loop {
        unsafe { asm!("hlt"); }
    }
}

extern "x86-interrupt" fn page_fault_handler(
    stack_frame: InterruptStackFrame,
    _error_code: PageFaultErrorCode,
) {
    println!("EXCEPTION: PAGE FAULT\n{:#?}", stack_frame);
    loop {
        unsafe { asm!("hlt"); }
    }
}

extern "x86-interrupt" fn gpf_handler(
    stack_frame: InterruptStackFrame,
    _error_code: u64,
) {
    println!("EXCEPTION: GENERAL PROTECTION FAULT\n{:#?}", stack_frame);
    loop {
        unsafe { asm!("hlt"); }
    }
}

/// Таймер PIT (IRQ0). Просто считает тики, ничего не печатает.
extern "x86-interrupt" fn timer_handler(_stack_frame: InterruptStackFrame) {
    static mut TICKS: u64 = 0;
    unsafe {
        TICKS += 1;
    }
    unsafe {
        PICS.lock().notify_end_of_interrupt(PIC_1_OFFSET);
    }
}

/// Клавиатура PS/2 (IRQ1). Учитывает Shift, кладёт символ в очередь.
extern "x86-interrupt" fn keyboard_handler(_stack_frame: InterruptStackFrame) {
    static mut SHIFT: bool = false;

    let mut port: Port<u8> = Port::new(0x60);
    let scancode: u8 = unsafe { port.read() };

    unsafe {
        match scancode {
            0x2A | 0x36 => SHIFT = true,   // Left/Right Shift press
            0xAA | 0xB6 => SHIFT = false,  // Left/Right Shift release
            _ => {
                if scancode & 0x80 == 0 {
                    if let Some(c) = scancode_to_ascii(scancode, SHIFT) {
                        KEY_QUEUE.lock().push_back(c);
                    }
                }
            }
        }
    }

    unsafe {
        PICS.lock().notify_end_of_interrupt(PIC_1_OFFSET + 1);
    }
}