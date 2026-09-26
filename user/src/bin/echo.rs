#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use userlib::*;

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    let args = unsafe { collect_args(argc, argv) };
    let mut buf = String::new();
    for (i, a) in args.iter().enumerate().skip(1) {
        if i > 1 { buf.push(' '); }
        buf.push_str(a);
    }
    buf.push('\n');
    write(buf.as_bytes());
    exit(0);
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { loop {} }