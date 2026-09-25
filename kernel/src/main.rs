#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]
#![feature(alloc_error_handler)]

extern crate alloc;

use core::arch::asm;

pub mod allocator;
pub mod cyrillic_font;
pub mod disk;
pub mod elf;
pub mod fat32;
pub mod framebuffer;
pub mod fs;
pub mod gdt;
pub mod gui;
pub mod interrupts;
pub mod keyboard;
pub mod memory;
pub mod mouse;
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

    let mut fa = unsafe { memory::BootInfoFrameAllocator::init(&boot_info.memory_regions) };
    serial_println!(
        "[boot] 5. frame_allocator created, regions = {}",
        boot_info.memory_regions.iter().count()
    );

    allocator::init_heap(&mut mapper, &mut fa).expect("heap init failed");
    serial_println!("[boot] 7. heap = {} bytes", allocator::HEAP_SIZE);

    *memory::FRAME_ALLOCATOR.lock() = Some(fa);

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

    unsafe {
        crate::gdt::init();
    }
    serial_println!("[boot] 8.5 GDT + TSS initialized");

    // ---- FAT32 mount + VFS init ----
    let fat32 = {
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
                Some(fat)
            }
            None => {
                serial_println!("[fat32] no filesystem on hda, skipping");
                None
            }
        }
    };
    crate::vfs::init(fat32);
    serial_println!("[vfs] initialized (RAMFS at /, FAT32 at C:)");

    crate::sched::init();
    serial_println!("[boot] 8.7 sched init (main + idle registered)");

    interrupts::init();
    serial_println!("[boot] 9. interrupts ready");

    crate::task::spawn(crate::task::clock::clock_task());
    crate::task::spawn(crate::task::repeat::repeat_task());
    serial_println!("[boot] 9.5 async tasks spawned");

    crate::sched::spawn("demo", demo_thread);
    serial_println!("[boot] 9.7 demo thread spawned");

    unsafe {
        crate::user::spawn_user_test(phys_offset);
    }
    serial_println!("[boot] 9.8 user thread spawned");

    serial_println!("[boot] 10. entering GUI");
    gui::run()
}

extern "C" fn demo_thread() -> ! {
    x86_64::instructions::interrupts::enable();
    let mut counter: u64 = 0;
    loop {
        counter += 1;
        serial_println!(
            "[demo] name={} id={} counter={}",
            crate::sched::current_name(),
            crate::sched::current_id(),
            counter,
        );
        crate::sched::sleep_ms(1000);
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    crate::serial_println!("PANIC: {}", info);
    if let Some(loc) = info.location() {
        crate::serial_println!("  at {}:{}", loc.file(), loc.line());
    }
    loop {
        unsafe { asm!("hlt"); }
    }
}