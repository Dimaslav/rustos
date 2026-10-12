#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use userlib::*;

fn parse_ip(s: &str) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut idx = 0;
    let mut cur: u32 = 0;
    let mut any = false;
    for c in s.chars() {
        if c == '.' {
            if idx >= 4 || !any { return None; }
            out[idx] = cur as u8;
            idx += 1;
            cur = 0;
            any = false;
        } else if c.is_ascii_digit() {
            cur = cur * 10 + (c as u32 - '0' as u32);
            if cur > 255 { return None; }
            any = true;
        } else {
            return None;
        }
    }
    if idx != 3 || !any { return None; }
    out[3] = cur as u8;
    Some(out)
}

fn ip_to_string(ip: [u8; 4]) -> String {
    format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3])
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    let args = unsafe { collect_args(argc, argv) };

    if args.len() < 2 {
        write(b"usage: ping IP\n");
        write(b"  example: ping 10.0.2.2\n");
        // Показать конфиг сети — полезно.
        if let Some((our, gw, dns, _mac)) = net_info() {
            let m = format!(
                "  our ip: {}, gateway: {}, dns: {}\n",
                ip_to_string(our), ip_to_string(gw), ip_to_string(dns)
            );
            write(m.as_bytes());
        }
        exit(1);
    }

    let ip = match parse_ip(&args[1]) {
        Some(ip) => ip,
        None => {
            write(b"ping: not an IP address\n");
            exit(1);
        }
    };

    let m = format!("PING {} (32 data bytes)\n", ip_to_string(ip));
    write(m.as_bytes());

    let mut ok = 0u32;
    let mut fail = 0u32;
    for i in 0..4u32 {
        match net_ping(ip, 2000) {
            Some(rtt) => {
                let m = format!(
                    "64 bytes from {}: icmp_seq={} time={} ms\n",
                    ip_to_string(ip), i, rtt
                );
                write(m.as_bytes());
                ok += 1;
            }
            None => {
                let m = format!("Request timeout for icmp_seq={}\n", i);
                write(m.as_bytes());
                fail += 1;
            }
        }
        if i < 3 { sleep_ms(500); }
    }

    let m = format!(
        "--- {} ping statistics ---\n\
         {} packets transmitted, {} received\n",
        ip_to_string(ip), ok + fail, ok
    );
    write(m.as_bytes());

    exit(if ok > 0 { 0 } else { 1 });
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { loop {} }