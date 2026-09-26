#![no_std]
#![no_main]

use userlib::*;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    write(b"[user] alive\n");
    sleep_ms(300);

    // Спавним shell в отдельном AS — он забирает клавиатуру себе
    // (set_focus(true)) и читает serial-ввод.
    write(b"[user] spawning shell\n");
    let shell_id = exec("shell");
    if shell_id == u64::MAX {
        write(b"[user] shell not found\n");
    } else {
        write(b"[user] shell spawned\n");
    }

    // Спавним калькулятор — он создаст своё окно поверх GUI.
    sleep_ms(200);
    write(b"[user] spawning calculator\n");
    let calc_id = spawn_calc();
    if calc_id == u64::MAX {
        write(b"[user] calc failed\n");
    } else {
        write(b"[user] calc spawned\n");
    }

    // Основной процесс просто спит в фоне.
    loop {
        sleep_ms(2000);
        write(b"[user] heartbeat\n");
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { loop {} }