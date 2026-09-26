#![no_std]
#![no_main]

use userlib::*;

const LINE_MAX: usize = 128;

/// Буфер строки — статический (в .bss), а не на стеке.
/// Компилятор не генерирует memset для .bss — загрузчик обнуляет его сам.
static mut LINE: [u8; LINE_MAX] = [0u8; LINE_MAX];
static mut LEN: usize = 0;

/// Syscall 11 через прямой inline asm (не полагаемся на userlib::set_focus).
#[inline(always)]
fn set_focus_inline(v: bool) {
    unsafe {
        core::arch::asm!(
            "int 0x80",
            inlateout("rax") 11u64 => _,
            in("rdi") if v { 1u64 } else { 0u64 },
            in("rsi") 0u64,
            in("rdx") 0u64,
            clobber_abi("sysv64"),
            options(nostack)
        );
    }
}

fn trim_in_place(s: &[u8]) -> &[u8] {
    let mut i = 0;
    while i < s.len() && (s[i] == b' ' || s[i] == b'\t') { i += 1; }
    let mut j = s.len();
    while j > i && (s[j - 1] == b' ' || s[j - 1] == b'\t') { j -= 1; }
    &s[i..j]
}

fn bytes_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() { return false; }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] { return false; }
        i += 1;
    }
    true
}

fn prompt() { write(b"> "); }

fn execute(s: &[u8]) {
    let s = trim_in_place(s);
    if s.is_empty() { return; }

    if bytes_eq(s, b"help") {
        write(b"commands:\n");
        write(b"  help       - this message\n");
        write(b"  calc       - spawn calculator window\n");
        write(b"  worker     - spawn worker thread\n");
        write(b"  hello      - say hello\n");
        write(b"  exit       - exit shell\n");
    } else if bytes_eq(s, b"calc") {
        let t = exec("calc");
        if t == u64::MAX { write(b"[shell] calc not found\n"); }
        else { write(b"[shell] calculator spawned\n"); }
    } else if bytes_eq(s, b"worker") {
        let t = exec("worker");
        if t == u64::MAX { write(b"[shell] worker not found\n"); }
        else { write(b"[shell] worker spawned\n"); }
    } else if bytes_eq(s, b"hello") {
        write(b"[shell] hello from Ring 3!\n");
    } else if bytes_eq(s, b"exit") {
        write(b"[shell] bye\n");
        set_focus_inline(false);
        exit(0);
    } else {
        write(b"unknown: ");
        write(s);
        write(b"\n");
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    write(b"[shell] started, type 'help'\n");
    set_focus_inline(true);
    write(b"[shell] focus set\n");

    // Явно обнуляем только те байты, что реально используем.
    // .bss уже нулевой, но на всякий случай.

    prompt();

    loop {
        let k = read_key();
        if k == u64::MAX {
            sleep_ms(30);
            continue;
        }
        let b = k as u8;
        unsafe {
            match b {
                0x0A => {
                    write(b"\n");
                    let len = LEN;
                    execute(&LINE[..len]);
                    LEN = 0;
                    prompt();
                }
                0x08 => {
                    if LEN > 0 {
                        LEN -= 1;
                        write(b"\x08 \x08");
                    }
                }
                c if (0x20..0x7F).contains(&c) => {
                    if LEN < LINE_MAX {
                        LINE[LEN] = c;
                        LEN += 1;
                        let buf = [c];
                        write(&buf);
                    }
                }
                _ => {}
            }
        }
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { loop {} }