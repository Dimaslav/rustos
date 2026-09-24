#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]
#![feature(alloc_error_handler)]

extern crate alloc;

use core::arch::asm;

pub mod allocator;
pub mod framebuffer;
pub mod gui;
pub mod interrupts;
pub mod keyboard;
pub mod mouse;
pub mod shell;
pub mod sound;
pub mod widgets;

use bootloader_api::config::Mapping;
use bootloader_api::{entry_point, BootInfo, BootloaderConfig};
use core::panic::PanicInfo;

pub static BOOTLOADER_CONFIG: BootloaderConfig = {
    let mut config = BootloaderConfig::new_default();
    config.mappings.physical_memory = Some(Mapping::Dynamic);
    config
};

entry_point!(kernel_main, config = &BOOTLOADER_CONFIG);

fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
    let phys_offset = boot_info
        .physical_memory_offset
        .into_option()
        .expect("physical_memory_offset не задан");

    // Heap — 4 МиБ, должно хватить даже на скромной конфигурации QEMU.
    allocator::init_heap(&boot_info.memory_regions, phys_offset);

    let fb = boot_info
        .framebuffer
        .as_mut()
        .expect("framebuffer отсутствует");
    let info = fb.info();
    let buffer = fb.buffer_mut();

    framebuffer::init(info, buffer);

    interrupts::init();

    gui::run();
}

/// Пишем панику в VGA-текст (0xB8000) — работает даже до framebuffer::init.
fn vga_panic_print(msg: &str) {
    const VGA: *mut u8 = 0xB8000 as *mut u8;
    let mut col = 0usize;
    for b in msg.bytes() {
        if col >= 80 { break; }
        let off = col * 2;
        unsafe {
            VGA.add(off).write_volatile(b);
            VGA.add(off + 1).write_volatile(0x4F);
        }
        col += 1;
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    vga_panic_print("PANIC! See framebuffer or serial.");

    let loc = info.location();
    if let Some(loc) = loc {
        let mut buf = [0u8; 80];
        let mut n = 0usize;
        for b in b"at " { if n < buf.len() { buf[n] = *b; n += 1; } }
        for b in loc.file().as_bytes() {
            if n < buf.len() - 8 { buf[n] = *b; n += 1; }
        }
        for b in b":" { if n < buf.len() { buf[n] = *b; n += 1; } }
        let mut line = loc.line();
        let mut tmp = [0u8; 8];
        let mut k = 0usize;
        if line == 0 { tmp[k] = b'0'; k += 1; }
        while line > 0 {
            tmp[k] = b'0' + (line % 10) as u8;
            line /= 10;
            k += 1;
        }
        while k > 0 {
            k -= 1;
            if n < buf.len() { buf[n] = tmp[k]; n += 1; }
        }

        const VGA: *mut u8 = 0xB8000 as *mut u8;
        for (i, b) in buf[..n].iter().enumerate() {
            if i >= 80 { break; }
            let off = 80 * 2 + i * 2;
            unsafe {
                VGA.add(off).write_volatile(*b);
                VGA.add(off + 1).write_volatile(0x4F);
            }
        }
    }

    loop {
        unsafe { asm!("hlt"); }
    }
}