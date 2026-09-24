//! PC speaker, неблокирующий. Включение — сразу, выключение — по tick().

use core::sync::atomic::{AtomicU64, Ordering};
use x86_64::instructions::port::Port;

static SPEAKER_OFF_AT: AtomicU64 = AtomicU64::new(0);

pub fn init() {}

/// Запустить сигнал. Не блокирует. Выключится автоматически через duration_ms.
pub fn beep(freq_hz: u32, duration_ms: u32) {
    if freq_hz == 0 || duration_ms == 0 {
        return;
    }
    let divisor = (1_193_182 / freq_hz) as u16;

    unsafe {
        let mut port43: Port<u8> = Port::new(0x43);
        let mut port42: Port<u8> = Port::new(0x42);
        port43.write(0xB6);
        port42.write((divisor & 0xFF) as u8);
        port42.write((divisor >> 8) as u8);

        let mut port61: Port<u8> = Port::new(0x61);
        let val = port61.read();
        port61.write(val | 0x03);
    }

    let now = crate::interrupts::ticks();
    let dur_ticks = ((duration_ms as u64 * 1822) / 100_000).max(1);
    SPEAKER_OFF_AT.store(now + dur_ticks, Ordering::Relaxed);
}

/// Вызывать из главного loop. Выключает speaker, когда истёк таймер.
pub fn tick() {
    let off_at = SPEAKER_OFF_AT.load(Ordering::Relaxed);
    if off_at == 0 {
        return;
    }
    let now = crate::interrupts::ticks();
    if now >= off_at {
        SPEAKER_OFF_AT.store(0, Ordering::Relaxed);
        unsafe {
            let mut port61: Port<u8> = Port::new(0x61);
            let val = port61.read();
            port61.write(val & !0x03);
        }
    }
}

pub fn click() { beep(1800, 30); }
pub fn open()  { beep(880, 60); }
pub fn error() { beep(220, 150); }