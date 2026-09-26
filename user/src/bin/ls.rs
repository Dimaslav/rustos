#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use userlib::*;

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    let args = unsafe { collect_args(argc, argv) };
    let path = if args.len() >= 2 {
        args[1].clone()
    } else {
        String::from("/")
    };

    let entries = list(&path, 64);
    if entries.is_empty() {
        write(b"(empty or not a dir)\n");
        exit(0);
    }
    for e in &entries {
        let line = if e.is_dir != 0 {
            format!("{}/\n", e.name_str())
        } else {
            format!("{}  ({} B)\n", e.name_str(), e.size)
        };
        write(line.as_bytes());
    }
    exit(0);
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { loop {} }