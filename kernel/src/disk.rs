//! Драйвер ATA PIO для чтения/записи секторов (LBA28).

use spin::Mutex;
use x86_64::instructions::port::Port;

pub const SECTOR_SIZE: usize = 512;

pub struct AtaDrive {
    io_base: u16,
    #[allow(dead_code)] // для будущего soft-reset через control port
    ctrl_base: u16,
    slave: bool,
}

impl AtaDrive {
    pub const fn new(io_base: u16, ctrl_base: u16, slave: bool) -> Self {
        Self { io_base, ctrl_base, slave }
    }

    fn status(&self) -> u8 {
        unsafe { Port::<u8>::new(self.io_base + 7).read() }
    }

    fn wait_not_busy(&self) {
        for _ in 0..1_000_000 {
            let s = self.status();
            if s & 0x80 == 0 {
                break;
            }
        }
    }

    fn wait_drq(&self) {
        for _ in 0..1_000_000 {
            let s = self.status();
            if s & 0x08 != 0 {
                return;
            }
            if s & 0x01 != 0 {
                return;
            }
        }
    }

    fn select_lba(&mut self, lba: u32, count: u8) {
        unsafe {
            let drive_head = 0xE0 | ((self.slave as u8) << 4) | (((lba >> 24) & 0x0F) as u8);
            Port::<u8>::new(self.io_base + 6).write(drive_head);

            // Небольшая задержка — 400 ns.
            for _ in 0..4 {
                let _ = Port::<u8>::new(self.io_base + 7).read();
            }

            Port::<u8>::new(self.io_base + 2).write(count);
            Port::<u8>::new(self.io_base + 3).write((lba & 0xFF) as u8);
            Port::<u8>::new(self.io_base + 4).write(((lba >> 8) & 0xFF) as u8);
            Port::<u8>::new(self.io_base + 5).write(((lba >> 16) & 0xFF) as u8);
        }
    }

    /// Читает один сектор по LBA.
    pub fn read_sector(&mut self, lba: u32) -> Option<[u8; SECTOR_SIZE]> {
        self.wait_not_busy();
        self.select_lba(lba, 1);
        unsafe {
            Port::<u8>::new(self.io_base + 7).write(0x20);
        }
        self.wait_drq();

        let s = self.status();
        if s & 0x01 != 0 {
            return None;
        }

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

    /// Пишет один сектор по LBA.
    pub fn write_sector(&mut self, lba: u32, data: &[u8; SECTOR_SIZE]) -> bool {
        self.wait_not_busy();
        self.select_lba(lba, 1);
        unsafe {
            Port::<u8>::new(self.io_base + 7).write(0x30);
        }
        self.wait_drq();

        let s = self.status();
        if s & 0x01 != 0 {
            return false;
        }

        unsafe {
            let mut port: Port<u16> = Port::new(self.io_base);
            for i in 0..(SECTOR_SIZE / 2) {
                let w = (data[i * 2] as u16) | ((data[i * 2 + 1] as u16) << 8);
                port.write(w);
            }
            // Flush
            Port::<u8>::new(self.io_base + 7).write(0xE7);
        }
        true
    }
}

/// Первичный master-диск: io_base=0x1F0, ctrl=0x3F6.
pub static PRIMARY_MASTER: Mutex<AtaDrive> =
    Mutex::new(AtaDrive::new(0x1F0, 0x3F6, false));

/// Первичный slave-диск.
pub static PRIMARY_SLAVE: Mutex<AtaDrive> =
    Mutex::new(AtaDrive::new(0x1F0, 0x3F6, true));

/// Вспомогательные функции для работы с большими объёмами данных.
pub fn read_bytes(drive: &mut AtaDrive, lba: u32, out: &mut [u8]) -> bool {
    let mut cur_lba = lba;
    let mut offset = 0;
    while offset < out.len() {
        let Some(sector) = drive.read_sector(cur_lba) else {
            return false;
        };
        let take = (out.len() - offset).min(SECTOR_SIZE);
        out[offset..offset + take].copy_from_slice(&sector[..take]);
        offset += take;
        cur_lba += 1;
    }
    true
}