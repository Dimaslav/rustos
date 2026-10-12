#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use userlib::*;

  //lkholikhuyk
  
#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    let args = unsafe { collect_args(argc, argv) };
    if args.len() < 2 {
        write(b"usage: kill PID [PID...]\n");
        exit(1);
    }

    let mut exit_code = 0u64;
    for s in args.iter().skip(1) {
        let pid: u64 = match s.parse() {
            Ok(v) => v,
            Err(_) => {
                write(b"kill: not a pid: ");
                write(s.as_bytes());
                write(b"\n");
                exit_code = 1;
                continue;
            }
        };
        if !kill(pid) {
            let m = format!("kill: no such process: {}\n", pid);
            write(m.as_bytes());
            exit_code = 1;
        }
    }
    exit(exit_code);
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { loop {} }