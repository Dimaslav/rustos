#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::vec::Vec;
use userlib::*;

fn count_and_print(name: &str, data: &[u8]) {
    let mut lines = 0u64;
    let mut words = 0u64;
    let mut in_word = false;
    for &b in data {
        if b == b'\n' { lines += 1; }
        let is_space = b == b' ' || b == b'\t' || b == b'\n' || b == b'\r';
        if is_space {
            in_word = false;
        } else if !in_word {
            words += 1;
            in_word = true;
        }
    }
    let s = format!("{:>7} {:>7} {:>7} {}\n",
                    lines, words, data.len(), name);
    write(s.as_bytes());
}

fn slurp(fd: u64) -> Vec<u8> {
    let mut data: Vec<u8> = Vec::new();
    let mut buf = [0u8; 1024];
    loop {
        let n = read(fd, buf.as_mut_ptr() as u64, buf.len() as u64);
        if n == 0 || n == u64::MAX { break; }
        data.extend_from_slice(&buf[..n as usize]);
    }
    data
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    let args = unsafe { collect_args(argc, argv) };

    if args.len() < 2 {
        let data = slurp(0);
        count_and_print("-", &data);
        exit(0);
    }

    let mut exit_code = 0u64;
    for path in args.iter().skip(1) {
        let fd = open(path);
        if fd == u64::MAX {
            write(b"wc: cannot open: ");
            write(path.as_bytes());
            write(b"\n");
            exit_code = 1;
            continue;
        }
        let data = slurp(fd);
        close(fd);
        count_and_print(path, &data);
    }
    exit(exit_code);
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { loop {} }