//! Драйвер ATA PIO (LBA28) + секторный кеш LRU + парсер MBR.
//!
//! SectorCache хранится в heap через Vec — раньше массив из 64 секторов
//! давал объект на ~33 KB, что переполняло стек kernel_main при создании
//! драйвера.

use alloc::vec::Vec;
use x86_64::instructions::port::Port;

pub const SECTOR_SIZE: usize = 512;
const CACHE_SIZE: usize = 64;

struct SectorCache {
    /// (lba, data). Пустое — если `lba == u32::MAX` (невалидный слот).
    entries: Vec<(u32, [u8; SECTOR_SIZE])>,
    age: Vec<u32>,
    tick: u32,
}

impl SectorCache {
    fn new() -> Self {
        let mut entries = Vec::with_capacity(CACHE_SIZE);
        let mut age = Vec::with_capacity(CACHE_SIZE);
        for _ in 0..CACHE_SIZE {
            entries.push((u32::MAX, [0u8; SECTOR_SIZE]));
            age.push(0);
        }
        Self { entries, age, tick: 0 }
    }

    fn next_tick(&mut self) -> u32 {
        self.tick = self.tick.wrapping_add(1);
        self.tick
    }

    fn lookup(&mut self, lba: u32) -> Option<usize> {
        for i in 0..CACHE_SIZE {
            if self.entries[i].0 == lba {
                let t = self.next_tick();
                self.age[i] = t;
                return Some(i);
            }
        }
        None
    }

    fn insert(&mut self, lba: u32, data: &[u8; SECTOR_SIZE]) {
        // Если lba уже есть — обновляем.
        for i in 0..CACHE_SIZE {
            if self.entries[i].0 == lba {
                self.entries[i].1.copy_from_slice(data);
                let t = self.next_tick();
                self.age[i] = t;
                return;
            }
        }
        // Свободный слот.
        for i in 0..CACHE_SIZE {
            if self.entries[i].0 == u32::MAX {
                self.entries[i].0 = lba;
                self.entries[i].1.copy_from_slice(data);
                let t = self.next_tick();
                self.age[i] = t;
                return;
            }
        }
        // LRU.
        let mut lru = 0usize;
        let mut lru_age = self.age[0];
        for i in 1..CACHE_SIZE {
            if self.age[i] < lru_age {
                lru_age = self.age[i];
                lru = i;
            }
        }
        self.entries[lru].0 = lba;
        self.entries[lru].1.copy_from_slice(data);
        let t = self.next_tick();
        self.age[lru] = t;
    }
}

pub struct AtaDrive {
    io_base: u16,
    #[allow(dead_code)]
    ctrl_base: u16,
    slave: bool,
    cache: SectorCache,
}

impl AtaDrive {
    pub fn new(io_base: u16, ctrl_base: u16, slave: bool) -> Self {
        Self {
            io_base,
            ctrl_base,
            slave,
            cache: SectorCache::new(),
        }
    }

    fn status(&self) -> u8 {
        unsafe { Port::<u8>::new(self.io_base + 7).read() }
    }

    fn wait_not_busy(&self) {
        for _ in 0..1_000_000 {
            let s = self.status();
            if s & 0x80 == 0 { break; }
        }
    }

    fn wait_drq(&self) {
        for _ in 0..1_000_000 {
            let s = self.status();
            if s & 0x08 != 0 { return; }
            if s & 0x01 != 0 { return; }
        }
    }

    fn select_lba(&mut self, lba: u32, count: u8) {
        unsafe {
            let drive_head = 0xE0 | ((self.slave as u8) << 4) | (((lba >> 24) & 0x0F) as u8);
            Port::<u8>::new(self.io_base + 6).write(drive_head);
            for _ in 0..4 {
                let _ = Port::<u8>::new(self.io_base + 7).read();
            }
            Port::<u8>::new(self.io_base + 2).write(count);
            Port::<u8>::new(self.io_base + 3).write((lba & 0xFF) as u8);
            Port::<u8>::new(self.io_base + 4).write(((lba >> 8) & 0xFF) as u8);
            Port::<u8>::new(self.io_base + 5).write(((lba >> 16) & 0xFF) as u8);
        }
    }

    fn real_read_sector(&mut self, lba: u32) -> Option<[u8; SECTOR_SIZE]> {
        self.wait_not_busy();
        self.select_lba(lba, 1);
        unsafe { Port::<u8>::new(self.io_base + 7).write(0x20); }
        self.wait_drq();
        let s = self.status();
        if s & 0x01 != 0 { return None; }
        let mut buf = [0u8; SECTOR_SIZE];
        unsafe {
            let mut data: Port<u16> = Port::new(self.io_base);
            for i in 0..(SECTOR_SIZE / 2) {
                let w = data.read();
                buf[i * 2] = (w & 0xFF) as u8;
                buf[i * 2 + 1] = (w >> 8) as u8;
            }
        }
        Some(buf)
    }

    fn real_write_sector(&mut self, lba: u32, data: &[u8; SECTOR_SIZE]) -> bool {
        self.wait_not_busy();
        self.select_lba(lba, 1);
        unsafe { Port::<u8>::new(self.io_base + 7).write(0x30); }
        self.wait_drq();
        let s = self.status();
        if s & 0x01 != 0 { return false; }
        unsafe {
            let mut port: Port<u16> = Port::new(self.io_base);
            for i in 0..(SECTOR_SIZE / 2) {
                let w = (data[i * 2] as u16) | ((data[i * 2 + 1] as u16) << 8);
                port.write(w);
            }
            Port::<u8>::new(self.io_base + 7).write(0xE7);
        }
        true
    }

    pub fn read_sector(&mut self, lba: u32) -> Option<[u8; SECTOR_SIZE]> {
        if let Some(idx) = self.cache.lookup(lba) {
            return Some(self.cache.entries[idx].1);
        }
        let buf = self.real_read_sector(lba)?;
        self.cache.insert(lba, &buf);
        Some(buf)
    }

    pub fn write_sector(&mut self, lba: u32, data: &[u8; SECTOR_SIZE]) -> bool {
        if !self.real_write_sector(lba, data) { return false; }
        self.cache.insert(lba, data);
        true
    }
}

pub fn read_bytes(drive: &mut AtaDrive, lba: u32, out: &mut [u8]) -> bool {
    let mut cur_lba = lba;
    let mut offset = 0;
    while offset < out.len() {
        let Some(sector) = drive.read_sector(cur_lba) else { return false; };
        let take = (out.len() - offset).min(SECTOR_SIZE);
        out[offset..offset + take].copy_from_slice(&sector[..take]);
        offset += take;
        cur_lba += 1;
    }
    true
}

// ---------- MBR ----------

#[derive(Debug, Clone, Copy)]
pub struct Partition {
    pub index: usize,
    pub bootable: bool,
    pub fs_type: u8,
    pub lba_start: u32,
    pub sectors: u32,
}

pub fn parse_mbr(drive: &mut AtaDrive) -> Option<Vec<Partition>> {
    let mbr = drive.read_sector(0)?;
    if mbr[510] != 0x55 || mbr[511] != 0xAA { return None; }
    let mut parts = Vec::new();
    for i in 0..4 {
        let off = 0x1BE + i * 16;
        let entry = &mbr[off..off + 16];
        let boot_flag = entry[0];
        let fs_type = entry[4];
        let lba_start = u32::from_le_bytes([entry[8], entry[9], entry[10], entry[11]]);
        let sectors = u32::from_le_bytes([entry[12], entry[13], entry[14], entry[15]]);
        if fs_type == 0 || lba_start == 0 { continue; }
        parts.push(Partition {
            index: i,
            bootable: boot_flag == 0x80,
            fs_type,
            lba_start,
            sectors,
        });
    }
    if parts.is_empty() { None } else { Some(parts) }
}

pub fn is_fat32_type(t: u8) -> bool {
    t == 0x0B || t == 0x0C
}