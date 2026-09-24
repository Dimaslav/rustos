//! Простейший драйвер Intel HDA: только «пищалка» — генерируем синус и
//! отправляем в поток DMA. Для полноценного звука нужен полноценный HDA-стек,
//! но для демонстрации "beep на клик" этого достаточно.

use spin::Mutex;
use x86_64::instructions::port::Port;

// Регистры HDA (MMIO) — берём из PCI BAR0. В QEMU обычно 0xFEBF0000.
// Для простоты работаем через известный адрес QEMU ICH9.
const HDA_BASE: u64 = 0xFEBF_0000;

// Регистры контроллера (offsets)
const GCAP: u64 = 0x00;
const GCTL: u64 = 0x08;
const STATESTS: u64 = 0x0E;
const INTCTL: u64 = 0x20;
const INTSTS: u64 = 0x24;

unsafe fn mmio_read32(off: u64) -> u32 {
    let p = (HDA_BASE + off) as *const u32;
    core::ptr::read_volatile(p)
}

unsafe fn mmio_write32(off: u64, v: u32) {
    let p = (HDA_BASE + off) as *mut u32;
    core::ptr::write_volatile(p, v);
}

unsafe fn mmio_write16(off: u64, v: u16) {
    let p = (HDA_BASE + off) as *mut u16;
    core::ptr::write_volatile(p, v);
}

pub static READY: Mutex<bool> = Mutex::new(false);

pub fn init() {
    unsafe {
        let gcap = mmio_read32(GCAP);
        let _num_streams = (gcap & 0x0F00) >> 8;
        let _num_sdo = (gcap & 0x000F) + 1;

        // Сброс
        mmio_write32(GCTL, 0);
        // Ждём
        for _ in 0..100_000 { core::hint::spin_loop(); }
        mmio_write32(GCTL, 1);
        for _ in 0..100_000 { core::hint::spin_loop(); }
        // Снимаем сброс
        mmio_write32(GCTL, 0);

        *READY.lock() = true;
    }
}

/// Воспроизвести короткий сигнал с заданной частотой и длительностью (мс).
/// Реализовано как «пищалка» через PC speaker (порт 0x61) — самый надёжный
/// способ для QEMU без полноценного DMA-стрима.
pub fn beep(freq_hz: u32, duration_ms: u32) {
    // PC speaker: программируем PIT канал 2, потом включаем спикер.
    let divisor = (1_193_182 / freq_hz) as u16;

    unsafe {
        // Отправляем команду PIT канал 2 (0xB6), потом два байта делителя
        let mut port43: Port<u8> = Port::new(0x43);
        let mut port42: Port<u8> = Port::new(0x42);
        port43.write(0xB6);
        port42.write((divisor & 0xFF) as u8);
        port42.write((divisor >> 8) as u8);

        // Включаем спикер (порт 0x61, биты 0 и 1)
        let mut port61: Port<u8> = Port::new(0x61);
        let val = port61.read();
        port61.write(val | 0x03);

        // Задержка — крутимся на PIT-тиках
        let start = crate::interrupts::ticks();
        let wait_ticks = (duration_ms as u64 * 1822) / 100_000;
        while crate::interrupts::ticks().saturating_sub(start) < wait_ticks {
            core::hint::spin_loop();
        }

        // Выключаем
        port61.write(val & !0x03);
    }
}

pub fn click() { beep(1800, 30); }
pub fn open()  { beep(880, 60); }
pub fn error() { beep(220, 150); }