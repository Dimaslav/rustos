#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use userlib::*;

#[no_mangle]
pub extern "C" fn _start(_argc: u64, _argv: *const *const u8) -> ! {
    // env — выводит «окружение» (пока хардкод, но с реальными значениями).
    let lines = [
        format!("PATH=builtin"),
        format!("PWD={}", getcwd_str()),
        format!("PID={}", getpid()),
        format!("PPID={}", getppid()),
        format!("SHELL=shell"),
        format!("OS=RustOS"),
        format!("ARCH=x86_64"),
    ];
    for l in &lines {
        write(l.as_bytes());
        write(b"\n");
    }
    exit(0);
}

fn getcwd_str() -> alloc::string::String {
    // Получаем cwd через /proc/self/status — там нет cwd, значит
    // возвращаем заглушку. В будущем сделаем syscall getcwd.
    alloc::string::String::from("/")
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { loop {} }