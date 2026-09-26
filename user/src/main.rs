#![no_std]
#![no_main]

extern crate alloc;

use userlib::*;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    write(b"[user] alive\n");

    // ---------- Тест аллокатора ----------
    {
        use alloc::format;
        use alloc::string::String;
        use alloc::vec::Vec;

        let mut v: Vec<String> = Vec::new();
        for i in 0..100u32 {
            v.push(format!("item-{}", i));
        }
        let msg = format!("[user] vec len = {}\n", v.len());
        write(msg.as_bytes());

        // Проверим, что первый и последний элементы корректны.
        if v.first().map(|s| s.as_str()) == Some("item-0")
            && v.last().map(|s| s.as_str()) == Some("item-99")
        {
            write(b"[user] alloc test OK\n");
        } else {
            write(b"[user] alloc test FAIL\n");
        }
    }
    // ---------- /Тест аллокатора ----------

    sleep_ms(300);

    write(b"[user] spawning shell\n");
    let shell_id = exec("shell");
    if shell_id == u64::MAX {
        write(b"[user] shell not found\n");
    } else {
        write(b"[user] shell spawned\n");
    }

    sleep_ms(200);
    write(b"[user] spawning calculator\n");
    let calc_id = spawn_calc();
    if calc_id == u64::MAX {
        write(b"[user] calc failed\n");
    } else {
        write(b"[user] calc spawned\n");
    }

    loop {
        sleep_ms(2000);
        write(b"[user] heartbeat\n");
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    write(b"[user] panic!\n");
    loop {}
}