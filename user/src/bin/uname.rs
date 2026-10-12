#![no_std]
#![no_main]

use userlib::*;

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    extern crate alloc;
    let args = unsafe { collect_args(argc, argv) };

    // uname без аргументов или с -a: полная строка.
    // uname -r: только версия ядра.
    if args.len() >= 2 && args[1] == "-r" {
        write(b"0.9.0\n");
    } else {
        write(b"RustOS 0.9.0 x86_64\n");
    }
    exit(0);
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { loop {} }