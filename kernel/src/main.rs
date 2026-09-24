#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]
#![feature(alloc_error_handler)]

extern crate alloc;

use core::arch::asm;

pub mod allocator;
pub mod disk;
pub mod fat32;
pub mod framebuffer;
pub mod fs;
pub mod gui;
pub mod interrupts;
pub mod keyboard;
pub mod memory;
pub mod mouse;
pub mod serial;
pub mod sound;
pub mod widgets;

use bootloader_api::config::{BootloaderConfig, Mapping};
use bootloader_api::{entry_point, BootInfo};
use core::panic::PanicInfo;
use x86_64::VirtAddr;

pub static BOOTLOADER_CONFIG: BootloaderConfig = {
    let mut config = BootloaderConfig::new_default();
    config.frame_buffer.minimum_framebuffer_width = Some(1920);
    config.frame_buffer.minimum_framebuffer_height = Some(1080);
    config.mappings.physical_memory = Some(Mapping::Dynamic);
    config
};

entry_point!(kernel_main, config = &BOOTLOADER_CONFIG);

fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
    serial::init();
    serial_println!("[boot] 1. serial online");

    let phys_opt = boot_info.physical_memory_offset.into_option();
    serial_println!("[boot] 2. phys_offset = {:?}", phys_opt);

    let phys_offset = VirtAddr::new(phys_opt.expect("physical_memory_offset не задан"));
    serial_println!("[boot] 3. phys_offset = {:#x}", phys_offset.as_u64());

    if let Some(fb) = boot_info.framebuffer.as_ref() {
        let info = fb.info();
        serial_println!(
            "[boot] framebuffer actual: {}x{} stride={} bpp={} format={:?}",
            info.width, info.height, info.stride, info.bytes_per_pixel, info.pixel_format
        );
    }

    for (i, r) in boot_info.memory_regions.iter().enumerate() {
        serial_println!(
            "[boot]   region[{}]: {:#x}..{:#x} kind={:?} len={:#x}",
            i, r.start, r.end, r.kind, r.end - r.start
        );
    }

    let mut mapper = unsafe { memory::init(phys_offset) };
    serial_println!("[boot] 4. mapper created");

    let mut frame_allocator =
        unsafe { memory::BootInfoFrameAllocator::init(&boot_info.memory_regions) };
    serial_println!(
        "[boot] 5. frame_allocator created, regions = {}",
        boot_info.memory_regions.iter().count()
    );

    allocator::init_heap(&mut mapper, &mut frame_allocator)
        .expect("heap init failed");
    serial_println!("[boot] 7. heap = {} bytes", allocator::HEAP_SIZE);

    let fb = boot_info
        .framebuffer
        .as_mut()
        .expect("framebuffer отсутствует");
    let info = fb.info();
    let buffer = fb.buffer_mut();
    framebuffer::init(info, buffer);
    serial_println!(
        "[boot] 8. framebuffer {}x{} {}bpp",
        info.width, info.height, info.bytes_per_pixel
    );

    // ---- FAT32: пробуем смонтировать первичный master ----
    {
        use crate::disk::AtaDrive;
        let drive = AtaDrive::new(0x1F0, 0x3F6, false);
        match fat32::Fat32::mount(drive, 0) {
            Some(mut fat) => {
                let entries = fat.list_root();
                serial_println!("[fat32] mounted, {} root entries", entries.len());
                for e in &entries {
                    serial_println!(
                        "[fat32]   {}{} ({} B)",
                        e.name,
                        if e.kind == fat32::FatKind::Directory { "/" } else { "" },
                        e.size
                    );
                }
                *gui::DISK.lock() = Some(fat);
            }
            None => {
                serial_println!("[fat32] no filesystem on hda, skipping");
            }
        }
    }

    interrupts::init();
    serial_println!("[boot] 9. interrupts ready");

    serial_println!("[boot] 10. entering GUI");
    gui::run()
}

fn vga_panic_print(msg: &str) {
    const VGA: *mut u8 = 0xB8000 as *mut u8;
    let mut col = 0usize;
    for b in msg.bytes() {
        if col >= 80 {
            break;
        }
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
        for b in b"at " {
            if n < buf.len() { buf[n] = *b; n += 1; }
        }
        for b in loc.file().as_bytes() {
            if n < buf.len() - 8 { buf[n] = *b; n += 1; }
        }
        for b in b":" {
            if n < buf.len() { buf[n] = *b; n += 1; }
        }
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