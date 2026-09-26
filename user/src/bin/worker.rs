#![no_std]
#![no_main]

use userlib::*;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    write(b"[worker] started\n");
    for i in 1..=10 {
        sleep_ms(1000);
        match i {
            1 => write(b"[worker] tick 1\n"),
            2 => write(b"[worker] tick 2\n"),
            3 => write(b"[worker] tick 3\n"),
            4 => write(b"[worker] tick 4\n"),
            5 => write(b"[worker] tick 5\n"),
            6 => write(b"[worker] tick 6\n"),
            7 => write(b"[worker] tick 7\n"),
            8 => write(b"[worker] tick 8\n"),
            9 => write(b"[worker] tick 9\n"),
            _ => write(b"[worker] tick 10\n"),
        }
    }
    write(b"[worker] exiting\n");
    exit(0);
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { loop {} }