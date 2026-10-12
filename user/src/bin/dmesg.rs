#![no_std]
#![no_main]

extern crate alloc;

use userlib::*;

#[no_mangle]
pub extern "C" fn _start(_argc: u64, _argv: *const *const u8) -> ! {
    let fd = open("/proc/kmsg");
    if fd == u64::MAX {
        write(b"dmesg: cannot open /proc/kmsg\n");
        exit(1);
    }
    let mut buf = [0u8; 1024];
    loop {
        let n = read(fd, buf.as_mut_ptr() as u64, buf.len() as u64);
        if n == 0 || n == u64::MAX { break; }
        write(&buf[..n as usize]);
    }
    close(fd);
    exit(0);
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { loop {} }