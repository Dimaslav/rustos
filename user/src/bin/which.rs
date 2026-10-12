#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use userlib::*;

const KNOWN: &[&str] = &[
    "init", "shell", "echo", "ls", "cat", "ps",
    "cp", "wc", "head", "dmesg", "kill", "uname", "which",
    "calc", "worker",
];

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    let args = unsafe { collect_args(argc, argv) };
    if args.len() < 2 {
        write(b"usage: which PROG...\n");
        exit(1);
    }

    let mut exit_code = 0u64;
    for name in args.iter().skip(1) {
        // Сейчас у нас «виртуальный PATH» — просто список встроенных
        // программ, потому что настоящей ФС /bin с исполняемыми нет.
        let found = KNOWN.iter().any(|k| *k == name.as_str());
        if found {
            let s = format!("{}: builtin\n", name);
            write(s.as_bytes());
        } else {
            let s = format!("{}: not found\n", name);
            write(s.as_bytes());
            exit_code = 1;
        }
    }
    exit(exit_code);
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { loop {} }