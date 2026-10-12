//! Минимальный PCI-сканер (config space через 0xCF8/0xCFC).

use alloc::vec::Vec;
use x86_64::instructions::port::Port;

#[derive(Debug, Clone, Copy)]
pub struct PciDevice {
    pub bus: u8,
    pub slot: u8,
    pub func: u8,
    pub vendor: u16,
    pub device: u16,
    pub class: u8,
    pub subclass: u8,
    pub bar: [u32; 6],
}

fn cfg_addr(bus: u8, slot: u8, func: u8, offset: u8) -> u32 {
    0x8000_0000u32
        | ((bus as u32) << 16)
        | ((slot as u32) << 11)
        | ((func as u32) << 8)
        | ((offset as u32) & 0xFC)
}

fn cfg_read_dword(bus: u8, slot: u8, func: u8, offset: u8) -> u32 {
    let addr = cfg_addr(bus, slot, func, offset);
    unsafe {
        let mut addr_port: Port<u32> = Port::new(0xCF8);
        let mut data_port: Port<u32> = Port::new(0xCFC);
        addr_port.write(addr);
        data_port.read()
    }
}

fn cfg_write_dword(bus: u8, slot: u8, func: u8, offset: u8, value: u32) {
    let addr = cfg_addr(bus, slot, func, offset);
    unsafe {
        let mut addr_port: Port<u32> = Port::new(0xCF8);
        let mut data_port: Port<u32> = Port::new(0xCFC);
        addr_port.write(addr);
        data_port.write(value);
    }
}

fn cfg_read_word(bus: u8, slot: u8, func: u8, offset: u8) -> u16 {
    let d = cfg_read_dword(bus, slot, func, offset & !3);
    let shift = (offset & 2) * 8;
    ((d >> shift) & 0xFFFF) as u16
}

/// Найти все устройства на шине.
pub fn scan_all() -> Vec<PciDevice> {
    let mut out = Vec::new();
    for bus in 0..=255u8 {
        for slot in 0..32u8 {
            for func in 0..8u8 {
                let vendor = cfg_read_word(bus, slot, func, 0x00);
                if vendor == 0xFFFF {
                    if func == 0 { break; }
                    continue;
                }
                let device = cfg_read_word(bus, slot, func, 0x02);
                let class_reg = cfg_read_dword(bus, slot, func, 0x08);
                let class = (class_reg >> 24) as u8;
                let subclass = (class_reg >> 16) as u8;

                let mut bar = [0u32; 6];
                for i in 0..6 {
                    bar[i] = cfg_read_dword(bus, slot, func, 0x10 + (i as u8) * 4);
                }

                out.push(PciDevice { bus, slot, func, vendor, device, class, subclass, bar });

                let header_type = cfg_read_word(bus, slot, 0, 0x0E);
                if func == 0 && header_type & 0x80 == 0 { break; }
            }
        }
    }
    out
}

/// Все устройства класса 0x04 (Multimedia), подкласс 0x01 (Audio).
pub fn find_audio() -> Vec<PciDevice> {
    scan_all()
        .into_iter()
        .filter(|d| d.class == 0x04 && d.subclass == 0x01)
        .collect()
}

/// Включить Bus Master и Memory Space в командном регистре.
pub fn enable_bus_master(dev: &PciDevice) {
    let cmd_off = 0x04;
    let d = cfg_read_dword(dev.bus, dev.slot, dev.func, cmd_off);
    let new = d | 0x0000_0006;
    cfg_write_dword(dev.bus, dev.slot, dev.func, cmd_off, new);
}

/// Определить размер MMIO BAR: пишем все единицы, читаем маску,
/// восстанавливаем оригинал.
///
/// Возвращает 0, если BAR не MMIO или недоступен.
pub fn bar_size_mmio(dev: &PciDevice, bar_idx: u8) -> u64 {
    if bar_idx >= 6 { return 0; }
    let off = 0x10 + bar_idx * 4;
    let orig = cfg_read_dword(dev.bus, dev.slot, dev.func, off);
    if orig & 1 != 0 {
        // I/O BAR, а не MMIO.
        return 0;
    }
    // Записать все единицы.
    cfg_write_dword(dev.bus, dev.slot, dev.func, off, 0xFFFF_FFFF);
    let mask = cfg_read_dword(dev.bus, dev.slot, dev.func, off);
    // Восстановить.
    cfg_write_dword(dev.bus, dev.slot, dev.func, off, orig);

    if mask == 0 || mask == 0xFFFF_FFFF {
        return 0;
    }
    // Для MMIO: младшие 4 бита — флаги (для memory BAR должны быть 0).
    // Размер = ~(mask & ~0xF) + 1.
    let masked = mask & 0xFFFF_FFF0;
    let size = (!masked).wrapping_add(1) as u64;
    // Sanity: размер должен быть степенью двойки.
    if size == 0 || (size & (size - 1)) != 0 {
        return 0;
    }
    size
}