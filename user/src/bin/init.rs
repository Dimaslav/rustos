#![no_std]
#![no_main]

use userlib::*;

/// Ручной вывод u64 в десятичном виде. Без `format!` — тот в этом
/// бинарнике почему-то приводит к `call 0x0`.
fn write_u64(mut n: u64) {
    if n == 0 {
        write(b"0");
        return;
    }
    let mut buf = [0u8; 20];
    let mut i = 20;
    while n > 0 && i > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    write(&buf[i..]);
}

#[no_mangle]
pub extern "C" fn _start(_argc: u64, _argv: *const *const u8) -> ! {
    write(b"[init] started (pid=");
    write_u64(getpid());
    write(b")\n");

    let mut restart_count: u64 = 0;

    loop {
        let child_pid = fork();

        if child_pid == u64::MAX {
            write(b"[init] fork() failed, retry in 1s\n");
            sleep_ms(1000);
            continue;
        }

        if child_pid == 0 {
            let _ = exec_replace("shell");
            write(b"[init] exec_replace(\"shell\") failed\n");
            exit(1);
        }

        restart_count += 1;
        write(b"[init] shell started (pid=");
        write_u64(child_pid);
        write(b", restart #");
        write_u64(restart_count);
        write(b")\n");

        let code = wait(child_pid);

        match code {
            Some(0) => {
                write(b"[init] shell exited cleanly, stopping\n");
                exit(0);
            }
            Some(c) => {
                write(b"[init] shell exited with code ");
                write_u64(c as u64);
                write(b", restarting\n");
            }
            None => {
                write(b"[init] wait() failed, restarting\n");
            }
        }

        sleep_ms(500);
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    write(b"[init] PANIC\n");
    loop {}
}