#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]
#![feature(alloc_error_handler)]

extern crate alloc;

use core::arch::asm;

pub mod allocator;
pub mod bmp;
pub mod cyrillic_font;
pub mod disk;
pub mod elf;
pub mod fat32;
pub mod fd;
pub mod framebuffer;
pub mod fs;
pub mod gdt;
pub mod gui;
pub mod image;
pub mod interrupts;
pub mod keyboard;
pub mod log;
pub mod memory;
pub mod mouse;
pub mod pipe;
pub mod png;
pub mod power;
pub mod sched;
pub mod serial;
pub mod sound;
pub mod syscall;
pub mod task;
pub mod user;
pub mod vfs;
pub mod widgets;
pub mod win;

#[cfg(feature = "headless_test")]
pub mod tests;

use bootloader_api::config::{BootloaderConfig, Mapping};
use bootloader_api::{entry_point, BootInfo};
use core::panic::PanicInfo;
use x86_64::VirtAddr;

#[allow(deprecated)]
pub static BOOTLOADER_CONFIG: BootloaderConfig = {
    let mut config = BootloaderConfig::new_default();
    config.frame_buffer.minimum_framebuffer_width = Some(1920);
    config.frame_buffer.minimum_framebuffer_height = Some(1080);
    config.mappings.physical_memory = Some(Mapping::Dynamic);
    config
};

entry_point!(kernel_main, config = &BOOTLOADER_CONFIG);

/// Аварийный вывод в COM1 — работает даже до `serial::init`.
unsafe fn emergency(s: &str) {
    use x86_64::instructions::port::Port;
    let mut data: Port<u8> = Port::new(0x3F8);
    let mut lsr: Port<u8> = Port::new(0x3FD);
    for b in s.bytes() {
        for _ in 0..1_000_000 {
            if lsr.read() & 0x20 != 0 { break; }
        }
        data.write(b);
    }
}

macro_rules! stage {
    ($s:expr) => {
        unsafe { emergency(concat!("[", $s, "]\n")) };
    };
}

fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
    stage!("K1");

    serial::init();

    log::set_level(if cfg!(feature = "headless_test") {
        log::Level::Debug
    } else {
        log::Level::Info
    });

    stage!("K2");
    log_info!("[boot] 1. serial online");

    let phys_opt = boot_info.physical_memory_offset.into_option();
    log_info!("[boot] 2. phys_offset = {:?}", phys_opt);

    let phys_offset = VirtAddr::new(
        phys_opt.expect("physical_memory_offset не задан"),
    );
    log_info!("[boot] 3. phys_offset = {:#x}", phys_offset.as_u64());

    let mut mapper = unsafe { memory::init(phys_offset) };
    let mut fa = unsafe { memory::BootInfoFrameAllocator::init(&boot_info.memory_regions) };

    allocator::init_heap(&mut mapper, &mut fa).expect("heap init failed");
    log_info!("[boot] 7. heap = {} bytes", allocator::HEAP_SIZE);

    *memory::FRAME_ALLOCATOR.lock() = Some(fa);

    let fb = boot_info
        .framebuffer
        .as_mut()
        .expect("framebuffer отсутствует");
    let info = fb.info();
    let buffer = fb.buffer_mut();
    framebuffer::init(info, buffer);
    log_info!(
        "[boot] 8. framebuffer {}x{} ({} bpp)",
        info.width, info.height, info.bytes_per_pixel * 8
    );

    unsafe { crate::gdt::init(); }
    log_info!("[boot] 8.5 GDT/TSS ready");

    // ---- FAT32 ----
    let fat32 = {
        use crate::disk::{AtaDrive, parse_mbr, is_fat32_type};
        let mut drive = AtaDrive::new(0x1F0, 0x3F6, false);
        let mut part_lba = 0u32;
        if let Some(parts) = parse_mbr(&mut drive) {
            log_info!("[mbr] found {} partitions", parts.len());
            if let Some(p) = parts.iter().find(|p| is_fat32_type(p.fs_type)) {
                part_lba = p.lba_start;
                log_info!("[mbr] mounting FAT32 at LBA {}", part_lba);
            } else {
                log_warn!("[mbr] FAT32 partition not found");
            }
        } else {
            log_warn!("[mbr] no MBR signature");
        }
        match fat32::Fat32::mount(drive, part_lba) {
            Some(mut fat) => {
                let entries = fat.list_root();
                log_info!("[fat32] mounted, {} root entries", entries.len());
                Some(fat)
            }
            None => {
                log_warn!("[fat32] no filesystem (running without disk)");
                None
            }
        }
    };
    crate::vfs::init(fat32);

    crate::sched::init();
    interrupts::init();
    log_info!("[boot] 9. interrupts ready");

    crate::task::spawn(crate::task::clock::clock_task());
    crate::task::spawn(crate::task::repeat::repeat_task());

    // ---------- HEADLESS TEST MODE ----------
    #[cfg(feature = "headless_test")]
    {
        crate::sched::sleep_ms(50);
        let failed = crate::tests::run_all();
        if failed == 0 {
            serial_println!("SMOKE_TEST_PASS");
        } else {
            serial_println!("SMOKE_TEST_FAIL");
        }
        crate::sched::sleep_ms(200);
        crate::power::shutdown();
    }

    // ---------- NORMAL GUI PATH ----------
    #[cfg(not(feature = "headless_test"))]
    {
        crate::sched::spawn("demo", demo_thread);
        match unsafe { crate::user::spawn_user_test(phys_offset) } {
            Ok(_id) => log_info!("[boot] 9.8 user thread spawned"),
            Err(e) => log_warn!("[boot] user thread not spawned: {:?}", e),
        }

        log_info!("[boot] 10. entering GUI");
        gui::run()
    }
}

#[cfg(not(feature = "headless_test"))]
extern "C" fn demo_thread() -> ! {
    x86_64::instructions::interrupts::enable();
    let mut counter: u64 = 0;
    loop {
        counter += 1;
        log_debug!("[demo] counter={}", counter);
        crate::sched::sleep_ms(1000);
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    unsafe { emergency("\n!!PANIC!!\n"); }
    crate::serial_println!("PANIC: {}", info);
    if let Some(loc) = info.location() {
        crate::serial_println!("  at {}:{}", loc.file(), loc.line());
    }
    #[cfg(feature = "headless_test")]
    crate::serial_println!("SMOKE_TEST_FAIL");
    loop {
        unsafe { asm!("hlt"); }
    }
}