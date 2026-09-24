//! Минимальная реализация FAT32: монтирование, листинг каталогов,
//! чтение файлов. Поддержка длинных имён (LFN) — базовая.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::format;

use crate::disk::{AtaDrive, SECTOR_SIZE};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FatKind {
    File,
    Directory,
}

#[derive(Clone)]
pub struct FatEntry {
    pub name: String,
    pub kind: FatKind,
    pub size: u32,
    pub first_cluster: u32,
}

pub struct Fat32 {
    pub drive: AtaDrive,
    pub part_lba: u32,
    bytes_per_sector: u16,
    sectors_per_cluster: u8,
    reserved_sectors: u16,
    num_fats: u8,
    fat_size_sectors: u32,
    root_cluster: u32,
    fs_info_sector: u16,
}

impl Fat32 {
    /// Монтирует FAT32 из раздела, начинающегося на `part_lba`.
    pub fn mount(drive: AtaDrive, part_lba: u32) -> Option<Self> {
        let mut drive = drive;
        let boot = drive.read_sector(part_lba)?;

        // Boot signature
        if boot[510] != 0x55 || boot[511] != 0xAA {
            return None;
        }

        let bytes_per_sector = u16::from_le_bytes([boot[0x0B], boot[0x0C]]);
        let sectors_per_cluster = boot[0x0D];
        let reserved_sectors = u16::from_le_bytes([boot[0x0E], boot[0x0F]]);
        let num_fats = boot[0x10];
        let fat_size_sectors = u32::from_le_bytes([
            boot[0x24], boot[0x25], boot[0x26], boot[0x27],
        ]);
        let root_cluster = u32::from_le_bytes([
            boot[0x2C], boot[0x2D], boot[0x2E], boot[0x2F],
        ]);
        let fs_info_sector = u16::from_le_bytes([boot[0x30], boot[0x31]]);

        if bytes_per_sector != 512 || sectors_per_cluster == 0 || num_fats == 0 {
            return None;
        }

        Some(Self {
            drive,
            part_lba,
            bytes_per_sector,
            sectors_per_cluster,
            reserved_sectors,
            num_fats,
            fat_size_sectors,
            root_cluster,
            fs_info_sector,
        })
    }

    fn fat_start_lba(&self) -> u32 {
        self.part_lba + self.reserved_sectors as u32
    }

    fn data_start_lba(&self) -> u32 {
        self.fat_start_lba()
            + self.num_fats as u32 * self.fat_size_sectors
    }

    fn cluster_to_lba(&self, cluster: u32) -> u32 {
        self.data_start_lba() + (cluster - 2) * self.sectors_per_cluster as u32
    }

    /// Читает FAT-запись для кластера.
    fn fat_next(&mut self, cluster: u32) -> u32 {
        let fat_offset = cluster * 4;
        let sector_in_fat = fat_offset / SECTOR_SIZE as u32;
        let offset_in_sector = (fat_offset % SECTOR_SIZE as u32) as usize;
        let lba = self.fat_start_lba() + sector_in_fat;

        let Some(sector) = self.drive.read_sector(lba) else {
            return 0x0FFF_FFFF;
        };
        let val = u32::from_le_bytes([
            sector[offset_in_sector],
            sector[offset_in_sector + 1],
            sector[offset_in_sector + 2],
            sector[offset_in_sector + 3],
        ]);
        val & 0x0FFF_FFFF
    }

    /// Читает все кластеры цепочки, начиная с `start`, возвращает сырые байты.
    /// Если `max_size` задан, читает не больше этого количества байт.
    fn read_chain(&mut self, start: u32, max_size: Option<u32>) -> Vec<u8> {
        let mut out = Vec::new();
        let mut cluster = start;
        let cluster_bytes = self.sectors_per_cluster as u32 * 512;
        let mut guard = 0u32;

        while cluster >= 2 && cluster < 0x0FFF_FFF8 && guard < 1_000_000 {
            guard += 1;
            let lba = self.cluster_to_lba(cluster);
            for s in 0..self.sectors_per_cluster as u32 {
                let Some(sector) = self.drive.read_sector(lba + s) else {
                    return out;
                };
                out.extend_from_slice(&sector);
                if let Some(max) = max_size {
                    if out.len() as u32 >= max {
                        out.truncate(max as usize);
                        return out;
                    }
                }
            }
            let next = self.fat_next(cluster);
            if next == cluster {
                break;
            }
            cluster = next;
            let _ = cluster_bytes;
        }
        out
    }

    /// Возвращает содержимое каталога (сырые байты всех кластеров).
    fn read_dir_raw(&mut self, start_cluster: u32) -> Vec<u8> {
        self.read_chain(start_cluster, None)
    }

    /// Разбирает содержимое каталога в список записей.
    fn parse_dir(&mut self, raw: &[u8]) -> Vec<FatEntry> {
        let mut out = Vec::new();
        let mut lfn_parts: Vec<String> = Vec::new();
        let mut i = 0usize;

        while i + 32 <= raw.len() {
            let e = &raw[i..i + 32];
            i += 32;

            if e[0] == 0x00 {
                break;
            }
            if e[0] == 0xE5 {
                lfn_parts.clear();
                continue;
            }

            let attr = e[11];
            if attr == 0x0F {
                // LFN entry
                let mut chars: [u16; 13] = [0; 13];
                let offsets = [1usize, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30];
                for (k, &off) in offsets.iter().enumerate() {
                    chars[k] = u16::from_le_bytes([e[off], e[off + 1]]);
                }
                let mut s = String::new();
                for &c in &chars {
                    if c == 0x0000 || c == 0xFFFF {
                        break;
                    }
                    if let Some(ch) = char::from_u32(c as u32) {
                        s.push(ch);
                    }
                }
                lfn_parts.push(s);
                continue;
            }

            let is_volume = attr & 0x08 != 0;
            if is_volume {
                lfn_parts.clear();
                continue;
            }

            let short = parse_short_name(&e[0..11]);
            let kind = if attr & 0x10 != 0 { FatKind::Directory } else { FatKind::File };
            let size = u32::from_le_bytes([e[28], e[29], e[30], e[31]]);
            let cl_hi = u16::from_le_bytes([e[20], e[21]]) as u32;
            let cl_lo = u16::from_le_bytes([e[26], e[27]]) as u32;
            let first_cluster = (cl_hi << 16) | cl_lo;

            let name = if lfn_parts.is_empty() {
                short
            } else {
                // LFN-части идут в обратном порядке
                lfn_parts.reverse();
                let mut s = String::new();
                for p in &lfn_parts {
                    s.push_str(p);
                }
                lfn_parts.clear();
                if s.is_empty() { short } else { s }
            };

            out.push(FatEntry { name, kind, size, first_cluster });
        }
        out
    }

    /// Листинг корневого каталога.
    pub fn list_root(&mut self) -> Vec<FatEntry> {
        let raw = self.read_dir_raw(self.root_cluster);
        self.parse_dir(&raw)
    }

    /// Листинг подкаталога.
    pub fn list_dir(&mut self, entry: &FatEntry) -> Vec<FatEntry> {
        if entry.kind != FatKind::Directory {
            return Vec::new();
        }
        let raw = self.read_dir_raw(entry.first_cluster);
        self.parse_dir(&raw)
    }

    /// Читает файл целиком.
    pub fn read_file(&mut self, entry: &FatEntry) -> Vec<u8> {
        if entry.kind != FatKind::File {
            return Vec::new();
        }
        self.read_chain(entry.first_cluster, Some(entry.size))
    }
}

fn parse_short_name(raw: &[u8]) -> String {
    let mut name = String::new();
    for &b in &raw[0..8] {
        if b == b' ' {
            break;
        }
        if b == 0 {
            break;
        }
        name.push((b as char).to_ascii_uppercase());
    }
    let mut ext = String::new();
    for &b in &raw[8..11] {
        if b == b' ' || b == 0 {
            break;
        }
        ext.push((b as char).to_ascii_lowercase());
    }
    if ext.is_empty() {
        name
    } else {
        format!("{}.{}", name, ext)
    }
}