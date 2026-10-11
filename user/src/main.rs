#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use userlib::*;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    write(b"T0\n");

    // Vec<u32>
    {
        let mut v: Vec<u32> = Vec::new();
        for i in 0..10u32 {
            v.push(i);
        }
        let sum: u32 = v.iter().sum();
        if sum != 45 {
            write(b"T1 vec FAIL sum\n");
            exit(1);
        }
        write(b"T1 vec u32 ok\n");
    }

    // Vec<String>
    {
        let mut v: Vec<String> = Vec::new();
        for i in 0..50u32 {
            let mut s = String::new();
            s.push_str("item");
            let _ = i;
            v.push(s);
        }
        if v.len() != 50 {
            write(b"T2 vec string FAIL len\n");
            exit(1);
        }
        write(b"T2 vec string ok\n");
    }

    // format!
    {
        let s = format!("test-{}", 42);
        write(b"T3 format: ");
        write(s.as_bytes());
        write(b"\n");
        if s.as_bytes() != b"test-42" {
            write(b"T3 format FAIL\n");
            exit(1);
        }
    }

    // Vec<u8> побольше — вдруг pool исчерпается
    {
        let mut big: Vec<u8> = Vec::new();
        for i in 0..20000usize {
            big.push((i & 0xFF) as u8);
        }
        if big.len() != 20000 {
            write(b"T3.5 big vec FAIL\n");
            exit(1);
        }
        write(b"T3.5 big vec 20000 ok\n");
    }

    write(b"T4 before exec\n");

    let id = exec("shell");

    write(b"T5 after exec\n");

    if id == u64::MAX {
        write(b"T6 shell failed\n");
    } else {
        write(b"T6 shell spawned\n");
    }

    // Второй spawn — calc
    sleep_ms(200);
    let cid = spawn_calc();
    if cid == u64::MAX {
        write(b"T6.5 calc failed\n");
    } else {
        write(b"T6.5 calc spawned\n");
    }

    loop {
        sleep_ms(1000);
        write(b"T7 alive\n");
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    write(b"[user] PANIC\n");
    loop {}
}