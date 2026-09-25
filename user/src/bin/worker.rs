#![no_std]
#![no_main]

use core::arch::asm;

#[inline(always)]
unsafe fn syscall(nr: u64, a1: u64, a2: u64, a3: u64) -> u64 {
    let ret: u64;
    asm!(
        "int 0x80",
        inlateout("rax") nr => ret,
        in("rdi") a1,
        in("rsi") a2,
        in("rdx") a3,
        options(nostack)
    );
    ret
}

fn write(buf: &[u8]) {
    unsafe { syscall(1, 1, buf.as_ptr() as u64, buf.len() as u64); }
}

fn sleep_ms(ms: u64) {
    unsafe { syscall(2, ms, 0, 0); }
}

fn exit(code: u64) -> ! {
    unsafe { syscall(0, code, 0, 0); }
    loop {}
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    write(b"[worker] started\n");
    for _ in 0..5 {
        sleep_ms(700);
        write(b"[worker] tick\n");
    }
    write(b"[worker] exiting\n");
    exit(0);
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}