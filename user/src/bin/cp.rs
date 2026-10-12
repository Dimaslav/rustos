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
    if args.len() < 3 {
        write(b"usage: cp SRC DST\n");
        exit(1);
    }
    let src = &args[1];
    let dst = &args[2];

    let fd_in = open(src);
    if fd_in == u64::MAX {
        write(b"cp: cannot open: ");
        write(src.as_bytes());
        write(b"\n");
        exit(1);
    }
    let data = slurp(fd_in);
    close(fd_in);

    let fd_out = open_write(dst, false);
    if fd_out == u64::MAX {
        write(b"cp: cannot create: ");
        write(dst.as_bytes());
        write(b"\n");
        exit(1);
    }
    let _ = write_fd(fd_out, &data);
    close(fd_out);
    exit(0);
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { loop {} }