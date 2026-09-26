#![no_std]
#![no_main]

extern crate alloc;

use userlib::*;

fn copy_fd_to_stdout(fd: u64) {
    let mut buf = [0u8; 1024];
    loop {
        let n = read(fd, buf.as_mut_ptr() as u64, buf.len() as u64);
        if n == 0 || n == u64::MAX { break; }
        write(&buf[..n as usize]);
    }
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    let args = unsafe { collect_args(argc, argv) };

    if args.len() < 2 {
        // Читаем stdin (может быть pipe).
        copy_fd_to_stdout(0);
        exit(0);
    }

    let mut exit_code = 0u64;
    for path in args.iter().skip(1) {
        let fd = open(path);
        if fd == u64::MAX {
            write(b"cat: not found: ");
            write(path.as_bytes());
            write(b"\n");
            exit_code = 1;
            continue;
        }
        copy_fd_to_stdout(fd);
        close(fd);
    }
    exit(exit_code);
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { loop {} }