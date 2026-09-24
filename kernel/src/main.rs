#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]
#![feature(alloc_error_handler)]

extern crate alloc;

use core::arch::asm;

pub mod allocator;
pub mod framebuffer;
pub mod interrupts;
pub mod keyboard;
pub mod shell;

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
    // 1. Прочитать phys_offset — это копия, не держит borrow
    let phys_offset = boot_info
        .physical_memory_offset
        .into_option()
        .expect("physical_memory_offset не задан");

    // 2. Инициализировать heap (использует memory_regions временно)
    allocator::init_heap(&boot_info.memory_regions, phys_offset);

    // 3. Только теперь забираем framebuffer —
    //    этот borrow останется на всю жизнь ядра.
    let fb = boot_info
        .framebuffer
        .as_mut()
        .expect("framebuffer отсутствует");
    let fb_info = fb.info();
    let fb_buffer = fb.buffer_mut();

    framebuffer::init(fb_info, fb_buffer);
    framebuffer::clear();

    println!("Rust OS v0.1.0");
    println!("===============");
    println!();
    println!("physical_memory_offset = {:#x}", phys_offset);
    println!("Heap: {} bytes initialized.", allocator::HEAP_SIZE);

    println!("Initializing interrupts...");
    interrupts::init();
    println!("IDT loaded, PIC remapped, interrupts enabled.");
    println!();

    shell::run();
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("PANIC: {}", info);
    loop {
        unsafe {
            asm!("hlt");
        }
    }
}