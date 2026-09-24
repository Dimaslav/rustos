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
pub mod memory;
pub mod mouse;
pub mod serial;
pub mod sound;
pub mod widgets;

use bootloader_api::config::Mapping;
use bootloader_api::{entry_point, BootInfo, BootloaderConfig};
use core::panic::PanicInfo;
use x86_64::VirtAddr;

pub static BOOTLOADER_CONFIG: BootloaderConfig = {
    let mut config = BootloaderConfig::new_default();
    config.mappings.physical_memory = Some(Mapping::Dynamic);
    config
};

entry_point!(kernel_main, config = &BOOTLOADER_CONFIG);

fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
    serial::init();
    serial_println!("[boot] Rust OS v0.5");

    let phys_offset = VirtAddr::new(
        boot_info
            .physical_memory_offset
            .into_option()
            .expect("physical_memory_offset не задан"),
    );
    let mut mapper = unsafe { memory::init(phys_offset) };
    let mut frame_allocator =
        unsafe { memory::BootInfoFrameAllocator::init(&boot_info.memory_regions) };

    allocator::init_heap(&mut mapper, &mut frame_allocator)
        .expect("Ошибка инициализации кучи");
    serial_println!("[boot] heap = {} bytes", allocator::HEAP_SIZE);

    let fb = boot_info
        .framebuffer
        .as_mut()
        .expect("framebuffer отсутствует");
    let info = fb.info();
    let buffer = fb.buffer_mut();
    framebuffer::init(info, buffer);
    serial_println!(
        "[boot] framebuffer {}x{} {}bpp",
        info.width, info.height, info.bytes_per_pixel
    );

    interrupts::init();
    serial_println!("[boot] interrupts ready");

    serial_println!("[boot] entering GUI");
    gui::run()
}

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
    serial_println!("PANIC: {}", info);
    vga_panic_print("PANIC! See serial for details.");

    if let Some(loc) = info.location() {
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