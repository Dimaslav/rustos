#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use userlib::*;

#[no_mangle]
pub extern "C" fn _start(_argc: u64, _argv: *const *const u8) -> ! {
    let procs = proc_list(64);
    write(b"PID  STATE     NAME\n");
    write(b"---  -----     ----\n");
    for p in &procs {
        let line = format!("{:<4} {:<9} {}\n", p.id, p.state_str(), p.name_str());
        write(line.as_bytes());
    }
    exit(0);
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { loop {} }