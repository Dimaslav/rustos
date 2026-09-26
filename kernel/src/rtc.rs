//! CMOS Real-Time Clock (ports 0x70 / 0x71).

use x86_64::instructions::port::Port;

#[derive(Clone, Copy)]
pub struct DateTime {
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub day: u8,
    pub month: u8,
    pub year: u8, // 0..=99 (20xx)
}

fn read_cmos(reg: u8) -> u8 {
    unsafe {
        let mut addr: Port<u8> = Port::new(0x70);
        let mut data: Port<u8> = Port::new(0x71);
        addr.write(reg);
        data.read()
    }
}

fn bcd_to_bin(b: u8) -> u8 {
    (b & 0x0F) + ((b >> 4) * 10)
}

pub fn read_rtc() -> DateTime {
    // Ждём, пока RTC не занят (bit 7 регистра 0x0A).
    for _ in 0..1_000_000 {
        if read_cmos(0x0A) & 0x80 == 0 { break; }
    }

    let second = read_cmos(0x00);
    let minute = read_cmos(0x02);
    let hour_raw = read_cmos(0x04);
    let day = read_cmos(0x07);
    let month = read_cmos(0x08);
    let year = read_cmos(0x09);
    let status_b = read_cmos(0x0B);

    let binary = status_b & 0x04 != 0;
    let h24 = status_b & 0x02 != 0;

    let conv = |b: u8| if binary { b } else { bcd_to_bin(b) };
    let s = conv(second);
    let m = conv(minute);
    let h_raw = hour_raw & 0x7F;
    let mut h = conv(h_raw);
    if !h24 {
        // 12-часовой формат.
        let pm = hour_raw & 0x80 != 0;
        if pm && h < 12 { h += 12; }
        if !pm && h == 12 { h = 0; }
    }
    let d = conv(day);
    let mo = conv(month);
    let y = conv(year);

    DateTime { hour: h, minute: m, second: s, day: d, month: mo, year: y }
}