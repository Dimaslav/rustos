#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use userlib::*;

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

    let mut n: usize = 10;
    let mut file: Option<&str> = None;
    let mut i = 1;
    while i < args.len() {
        if args[i] == "-n" && i + 1 < args.len() {
            n = args[i + 1].parse().unwrap_or(10);
            i += 2;
        } else if file.is_none() {
            file = Some(&args[i]);
            i += 1;
        } else {
            i += 1;
        }
    }

    let data: Vec<u8> = if let Some(path) = file {
        let fd = open(path);
        if fd == u64::MAX {
            write(b"head: cannot open: ");
            write(path.as_bytes());
            write(b"\n");
            exit(1);
        }
        let d = slurp(fd);
        close(fd);
        d
    } else {
        slurp(0)
    };

    let mut lines_out = 0usize;
    for &b in &data {
        if lines_out >= n { break; }
        let ch = [b];
        write(&ch);
        if b == b'\n' { lines_out += 1; }
    }
    exit(0);
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { loop {} }