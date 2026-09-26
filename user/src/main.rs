#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use userlib::*;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    write(b"[user] alive\n");

    // Шаг 1: одиночная String.
    {
        let mut s = String::new();
        s.push_str("hello");
        write(b"[user] 1 push_str ok\n");
    }

    // Шаг 2: Vec<u32>.
    {
        let mut v: Vec<u32> = Vec::new();
        for i in 0..10u32 { v.push(i); }
        write(b"[user] 2 vec_u32 ok\n");
    }

    // Шаг 3: Vec<String> без format!.
    {
        let mut vs: Vec<String> = Vec::new();
        for _ in 0..50 {
            let mut s = String::new();
            s.push_str("item");
            vs.push(s);
        }
        write(b"[user] 3 vec_string ok\n");
    }

    // Шаг 4: format!.
    {
        let s = alloc::format!("test-{}", 42);
        write(b"[user] 4 format ok\n");
        let _ = s;
    }

    // Шаг 5: спавн shell.
    write(b"[user] 5 spawning shell\n");
    let shell_id = exec("shell");
    if shell_id == u64::MAX {
        write(b"[user] shell not found\n");
    } else {
        write(b"[user] shell spawned\n");
    }

    sleep_ms(200);
    write(b"[user] 6 spawning calc\n");
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
    write(b"[user] PANIC\n");
    loop {}
}