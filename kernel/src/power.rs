//! Выключение и перезагрузка.
//! Для QEMU используем ACPI-совместимые порты.

use x86_64::instructions::port::Port;

/// Выключение через QEMU ACPI (PM1a_CNT с SLP_TYP=5, SLP_EN=1).
/// На реальном железе нужен полный ACPI-стек, но для QEMU этого достаточно.
pub fn shutdown() -> ! {
    crate::serial_println!("[power] shutdown requested");
    unsafe {
        // QEMU: 0x604 — i440fx PM1a_CNT; 0xB004 — более старая эмуляция.
        let mut p1: Port<u16> = Port::new(0x604);
        p1.write(0x2000);
        let mut p2: Port<u16> = Port::new(0xB004);
        p2.write(0x2000);
    }
    // Если не сработало — просто висим.
    loop {
        x86_64::instructions::hlt();
    }
}

/// Перезагрузка: пишем 0xFE в контроллер клавиатуры (i8042).
pub fn reboot() -> ! {
    crate::serial_println!("[power] reboot requested");
    unsafe {
        let mut p: Port<u8> = Port::new(0x64);
        p.write(0xFE);
    }
    loop {
        x86_64::instructions::hlt();
    }
}