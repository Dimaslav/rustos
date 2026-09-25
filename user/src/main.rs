#![no_std]
#![no_main]

use userlib::*;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    write(b"[user] alive\n");
    sleep_ms(300);

    write(b"[user] spawning calculator\n");
    let _ = spawn_calc();

    for _ in 0..3 {
        write(b"[user] tick\n");
        sleep_ms(900);
    }
    write(b"[user] exiting\n");
    exit(0);
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { loop {} }