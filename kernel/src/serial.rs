//! Простой COM1 logger для диагностики. QEMU: `-serial stdio`.

use core::fmt;
use spin::Mutex;
use x86_64::instructions::port::Port;

pub static SERIAL: Mutex<Option<Serial>> = Mutex::new(None);

pub struct Serial {
    data: Port<u8>,
    ier: Port<u8>,
    fcr: Port<u8>,
    lcr: Port<u8>,
    mcr: Port<u8>,
}

impl Serial {
    unsafe fn init() -> Self {
        let mut s = Serial {
            data: Port::new(0x3F8),
            ier: Port::new(0x3F9),
            fcr: Port::new(0x3FA),
            lcr: Port::new(0x3FB),
            mcr: Port::new(0x3FC),
        };
        s.ier.write(0x00); // disable interrupts
        s.lcr.write(0x80); // enable DLAB
        s.data.write(0x03); // divisor lo = 3 (38400 baud)
        s.ier.write(0x00); // divisor hi = 0
        s.lcr.write(0x03); // 8 bits, no parity, 1 stop
        s.fcr.write(0xC7); // enable FIFO, clear, 14-byte threshold
        s.mcr.write(0x0B); // IRQs enabled, RTS/DSR set
        s
    }

    fn write_byte(&mut self, byte: u8) {
        unsafe {
            // Ждём, пока transmit buffer пуст (bit 5 = 0x20)
            let mut lsr: Port<u8> = Port::new(0x3FD);
            for _ in 0..100_000 {
                if lsr.read() & 0x20 != 0 { break; }
            }
            self.data.write(byte);
        }
    }
}

impl fmt::Write for Serial {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for b in s.bytes() {
            self.write_byte(b);
        }
        Ok(())
    }
}

pub fn init() {
    let s = unsafe { Serial::init() };
    *SERIAL.lock() = Some(s);
}

pub fn _print(args: fmt::Arguments) {
    use core::fmt::Write;
    if let Some(s) = SERIAL.lock().as_mut() {
        let _ = s.write_fmt(args);
    }
}

#[macro_export]
macro_rules! serial_print {
    ($($arg:tt)*) => ($crate::serial::_print(format_args!($($arg)*)));
}

#[macro_export]
macro_rules! serial_println {
    () => ($crate::serial_print!("\n"));
    ($($arg:tt)*) => ($crate::serial_print!("{}\n", format_args!($($arg)*)));
}