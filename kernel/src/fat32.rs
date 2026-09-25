//! Минимальная реализация FAT32: монтирование, листинг каталогов,
//! чтение и запись файлов, создание/удаление каталогов.

use alloc::string::{String, ToString};
use alloc::vec;
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
    pub dir_offset: u32,
    pub entry_off: usize,
}

#[derive(Debug)]
pub enum FatError {
    Io,
    NoSpace,
    NotFound,
    AlreadyExists,
    InvalidName,
    NotADirectory,
}

pub type FatResult<T> = Result<T, FatError>;

const ATTR_VOLUME_ID: u8 = 0x08;
const ATTR_DIRECTORY: u8 = 0x10;
const ATTR_ARCHIVE: u8 = 0x20;
const ATTR_LFN: u8 = 0x0F;

const FAT_EOC: u32 = 0x0FFF_FFFF;

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
    pub fn mount(drive: AtaDrive, part_lba: u32) -> Option<Self> {
        let mut drive = drive;
        let boot = drive.read_sector(part_lba)?;
        if boot[510] != 0x55 || boot[511] != 0xAA {
            return None;
        }

        let bytes_per_sector = u16::from_le_bytes([boot[0x0B], boot[0x0C]]);
        let sectors_per_cluster = boot[0x0D];
        let reserved_sectors = u16::from_le_bytes([boot[0x0E], boot[0x0F]]);
        let num_fats = boot[0x10];
        let fat_size_sectors =
            u32::from_le_bytes([boot[0x24], boot[0x25], boot[0x26], boot[0x27]]);
        let root_cluster =
            u32::from_le_bytes([boot[0x2C], boot[0x2D], boot[0x2E], boot[0x2F]]);
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

    // ---------- Публичные геттеры ----------

    pub fn root_cluster(&self) -> u32 {
        self.root_cluster
    }

    // ---------- Низкоуровневые смещения ----------

    fn fat_start_lba(&self) -> u32 {
        self.part_lba + self.reserved_sectors as u32
    }

    fn data_start_lba(&self) -> u32 {
        self.fat_start_lba() + self.num_fats as u32 * self.fat_size_sectors
    }

    fn cluster_size_bytes(&self) -> u32 {
        self.sectors_per_cluster as u32 * self.bytes_per_sector as u32
    }

    fn cluster_to_lba(&self, cluster: u32) -> u32 {
        self.data_start_lba() + (cluster - 2) * self.sectors_per_cluster as u32
    }

    // ---------- FAT ----------

    fn fat_next(&mut self, cluster: u32) -> u32 {
        let fat_offset = cluster * 4;
        let sector_in_fat = fat_offset / SECTOR_SIZE as u32;
        let offset_in_sector = (fat_offset % SECTOR_SIZE as u32) as usize;
        let lba = self.fat_start_lba() + sector_in_fat;
        let Some(sector) = self.drive.read_sector(lba) else {
            return FAT_EOC;
        };
        let val = u32::from_le_bytes([
            sector[offset_in_sector],
            sector[offset_in_sector + 1],
            sector[offset_in_sector + 2],
            sector[offset_in_sector + 3],
        ]);
        val & 0x0FFF_FFFF
    }

    fn fat_set(&mut self, cluster: u32, value: u32) -> bool {
        let fat_offset = cluster * 4;
        let sector_in_fat = fat_offset / SECTOR_SIZE as u32;
        let offset_in_sector = (fat_offset % SECTOR_SIZE as u32) as usize;

        for copy in 0..self.num_fats as u32 {
            let lba = self.fat_start_lba() + copy * self.fat_size_sectors + sector_in_fat;
            let Some(mut sector) = self.drive.read_sector(lba) else {
                return false;
            };
            let old = u32::from_le_bytes([
                sector[offset_in_sector],
                sector[offset_in_sector + 1],
                sector[offset_in_sector + 2],
                sector[offset_in_sector + 3],
            ]);
            let v = (value & 0x0FFF_FFFF) | (old & 0xF000_0000);
            sector[offset_in_sector] = (v & 0xFF) as u8;
            sector[offset_in_sector + 1] = ((v >> 8) & 0xFF) as u8;
            sector[offset_in_sector + 2] = ((v >> 16) & 0xFF) as u8;
            sector[offset_in_sector + 3] = ((v >> 24) & 0xFF) as u8;
            if !self.drive.write_sector(lba, &sector) {
                return false;
            }
        }
        true
    }

    fn find_free_cluster(&mut self) -> Option<u32> {
        let total_fat_sectors = self.fat_size_sectors;
        for s in 0..total_fat_sectors {
            let lba = self.fat_start_lba() + s;
            let Some(sector) = self.drive.read_sector(lba) else {
                return None;
            };
            for i in (0..SECTOR_SIZE).step_by(4) {
                let val = u32::from_le_bytes([
                    sector[i],
                    sector[i + 1],
                    sector[i + 2],
                    sector[i + 3],
                ]) & 0x0FFF_FFFF;
                if val == 0 {
                    let cluster = (s * SECTOR_SIZE as u32 + i as u32) / 4;
                    if cluster >= 2 {
                        return Some(cluster);
                    }
                }
            }
        }
        None
    }

    fn free_chain(&mut self, start: u32) -> bool {
        let mut cluster = start;
        let mut guard = 0u32;
        while cluster >= 2 && cluster < 0x0FFF_FFF8 && guard < 1_000_000 {
            guard += 1;
            let next = self.fat_next(cluster);
            if !self.fat_set(cluster, 0) {
                return false;
            }
            if next == FAT_EOC || next == cluster {
                break;
            }
            cluster = next;
        }
        true
    }

    // ---------- Кластеры ----------

    fn read_cluster(&mut self, cluster: u32) -> Option<Vec<u8>> {
        let lba = self.cluster_to_lba(cluster);
        let mut buf = Vec::with_capacity(self.cluster_size_bytes() as usize);
        for s in 0..self.sectors_per_cluster as u32 {
            let sector = self.drive.read_sector(lba + s)?;
            buf.extend_from_slice(&sector);
        }
        Some(buf)
    }

    fn write_cluster(&mut self, cluster: u32, data: &[u8]) -> bool {
        let lba = self.cluster_to_lba(cluster);
        let csize = self.cluster_size_bytes() as usize;
        for s in 0..self.sectors_per_cluster as u32 {
            let mut sector = [0u8; SECTOR_SIZE];
            let off = s as usize * SECTOR_SIZE;
            let take = (csize - off).min(SECTOR_SIZE);
            if off + take <= data.len() {
                sector[..take].copy_from_slice(&data[off..off + take]);
            } else if off < data.len() {
                let n = data.len() - off;
                sector[..n].copy_from_slice(&data[off..off + n]);
            }
            if !self.drive.write_sector(lba + s, &sector) {
                return false;
            }
        }
        true
    }

    fn allocate_chain(&mut self, count: u32) -> Option<u32> {
        if count == 0 {
            return None;
        }
        let mut prev: Option<u32> = None;
        let mut first: Option<u32> = None;
        for _ in 0..count {
            let c = self.find_free_cluster()?;
            if !self.fat_set(c, FAT_EOC) {
                return None;
            }
            if let Some(p) = prev {
                self.fat_set(p, c);
            } else {
                first = Some(c);
            }
            prev = Some(c);
        }
        first
    }

    // ---------- Цепочки ----------

    fn read_chain(&mut self, start: u32, max_size: Option<u32>) -> Vec<u8> {
        let mut out = Vec::new();
        let mut cluster = start;
        let mut guard = 0u32;
        while cluster >= 2 && cluster < 0x0FFF_FFF8 && guard < 1_000_000 {
            guard += 1;
            let Some(chunk) = self.read_cluster(cluster) else {
                break;
            };
            out.extend_from_slice(&chunk);
            if let Some(max) = max_size {
                if out.len() as u32 >= max {
                    out.truncate(max as usize);
                    return out;
                }
            }
            let next = self.fat_next(cluster);
            if next == cluster || next >= 0x0FFF_FFF8 {
                break;
            }
            cluster = next;
        }
        out
    }

    // ---------- Публичные операции ----------

    pub fn list_root(&mut self) -> Vec<FatEntry> {
        let root = self.root_cluster;
        let raw = self.read_chain(root, None);
        let mut entries = self.parse_dir_bytes(&raw);
        self.annotate_positions(root, &mut entries);
        entries
    }

    pub fn list_dir(&mut self, entry: &FatEntry) -> Vec<FatEntry> {
        if entry.kind != FatKind::Directory {
            return Vec::new();
        }
        let start = entry.first_cluster;
        let raw = self.read_chain(start, None);
        let mut entries = self.parse_dir_bytes(&raw);
        self.annotate_positions(start, &mut entries);
        entries
    }

    pub fn read_file(&mut self, entry: &FatEntry) -> Vec<u8> {
        if entry.kind != FatKind::File {
            return Vec::new();
        }
        self.read_chain(entry.first_cluster, Some(entry.size))
    }

    pub fn find_in_dir(&mut self, dir_cluster: u32, name: &str) -> Option<FatEntry> {
        let raw = self.read_chain(dir_cluster, None);
        let mut entries = self.parse_dir_bytes(&raw);
        self.annotate_positions(dir_cluster, &mut entries);
        entries
            .into_iter()
            .find(|e| e.name.eq_ignore_ascii_case(name))
    }

    // ---------- Парсинг директории ----------

    fn parse_dir_bytes(&mut self, raw: &[u8]) -> Vec<FatEntry> {
        let mut out: Vec<FatEntry> = Vec::new();
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
            if attr == ATTR_LFN {
                let offsets = [
                    1usize, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30,
                ];
                let mut s = String::new();
                for &off in offsets.iter() {
                    let c = u16::from_le_bytes([e[off], e[off + 1]]);
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

            if attr & ATTR_VOLUME_ID != 0 {
                lfn_parts.clear();
                continue;
            }

            let short = parse_short_name(&e[0..11]);
            let kind = if attr & ATTR_DIRECTORY != 0 {
                FatKind::Directory
            } else {
                FatKind::File
            };
            let size = u32::from_le_bytes([e[28], e[29], e[30], e[31]]);
            let cl_hi = u16::from_le_bytes([e[20], e[21]]) as u32;
            let cl_lo = u16::from_le_bytes([e[26], e[27]]) as u32;
            let first_cluster = (cl_hi << 16) | cl_lo;

            let name = if lfn_parts.is_empty() {
                short
            } else {
                lfn_parts.reverse();
                let mut s = String::new();
                for p in &lfn_parts {
                    s.push_str(p);
                }
                lfn_parts.clear();
                if s.is_empty() {
                    short
                } else {
                    s
                }
            };

            out.push(FatEntry {
                name,
                kind,
                size,
                first_cluster,
                dir_offset: 0,
                entry_off: 0,
            });
        }
        out
    }

    /// Проставляет `dir_offset` (LBA сектора) и `entry_off` (смещение внутри сектора)
    /// для каждой записи, сканируя каталог повторно.
    fn annotate_positions(&mut self, start_cluster: u32, entries: &mut [FatEntry]) {
        let raw = self.read_chain(start_cluster, None);

        // Карта: индекс байта в raw → (LBA, offset внутри сектора).
        // Проходим по цепочке кластеров и собираем LBA для каждого сектора.
        let spc = self.sectors_per_cluster as u32;
        let mut sector_lbas: Vec<u32> = Vec::new();
        let mut c = start_cluster;
        let mut guard = 0u32;
        while c >= 2 && c < 0x0FFF_FFF8 && guard < 1_000_000 {
            guard += 1;
            let lba = self.cluster_to_lba(c);
            for s in 0..spc {
                sector_lbas.push(lba + s);
            }
            let next = self.fat_next(c);
            if next == c || next >= 0x0FFF_FFF8 {
                break;
            }
            c = next;
        }

        let mut i = 0usize;
        let mut entry_idx = 0usize;
        let mut lfn_count = 0usize;
        while i + 32 <= raw.len() {
            let e = &raw[i..i + 32];
            if e[0] == 0x00 {
                break;
            }
            if e[0] == 0xE5 {
                lfn_count = 0;
                i += 32;
                continue;
            }
            let attr = e[11];
            if attr == ATTR_LFN {
                lfn_count += 1;
                i += 32;
                continue;
            }
            if attr & ATTR_VOLUME_ID != 0 {
                lfn_count = 0;
                i += 32;
                continue;
            }

            // Реальная запись находится на `i - lfn_count*32`.
            let real_off = i - lfn_count * 32;
            let sector_in_chain = real_off / SECTOR_SIZE;
            let off_in_sector = real_off % SECTOR_SIZE;

            if entry_idx < entries.len() {
                if let Some(&lba) = sector_lbas.get(sector_in_chain) {
                    entries[entry_idx].dir_offset = lba;
                    entries[entry_idx].entry_off = off_in_sector;
                }
            }

            entry_idx += 1;
            lfn_count = 0;
            i += 32;
        }
    }

    // ---------- Создание файла ----------

    pub fn create_file(&mut self, dir_cluster: u32, name: &str) -> FatResult<FatEntry> {
        if !valid_name(name) {
            return Err(FatError::InvalidName);
        }
        if self.find_in_dir(dir_cluster, name).is_some() {
            return Err(FatError::AlreadyExists);
        }

        let (lba, off) = self.find_free_dir_slot(dir_cluster)?;
        let mut sector = self.drive.read_sector(lba).ok_or(FatError::Io)?;

        let (base, ext) = to_short_name(name);
        let mut e = [0u8; 32];
        e[0..8].copy_from_slice(&base);
        e[8..11].copy_from_slice(&ext);
        e[11] = ATTR_ARCHIVE;

        for i in 0..32 {
            sector[off + i] = e[i];
        }
        if !self.drive.write_sector(lba, &sector) {
            return Err(FatError::Io);
        }

        Ok(FatEntry {
            name: name.to_string(),
            kind: FatKind::File,
            size: 0,
            first_cluster: 0,
            dir_offset: lba,
            entry_off: off,
        })
    }

    fn find_free_dir_slot(&mut self, dir_cluster: u32) -> FatResult<(u32, usize)> {
        let csize = self.cluster_size_bytes() as usize;
        let entries_per_cluster = csize / 32;
        let mut cluster = dir_cluster;
        let mut guard = 0u32;

        while cluster >= 2 && cluster < 0x0FFF_FFF8 && guard < 1_000_000 {
            guard += 1;
            let lba = self.cluster_to_lba(cluster);
            for k in 0..entries_per_cluster {
                let off_in_cluster = k * 32;
                let sector_idx = off_in_cluster / SECTOR_SIZE;
                let off_in_sector = off_in_cluster % SECTOR_SIZE;
                let abs_lba = lba + sector_idx as u32;
                let sector = self.drive.read_sector(abs_lba).ok_or(FatError::Io)?;
                let first = sector[off_in_sector];
                if first == 0x00 || first == 0xE5 {
                    return Ok((abs_lba, off_in_sector));
                }
            }
            // Здесь `cluster` — последний посещённый.
            let next = self.fat_next(cluster);
            if next == cluster || next >= 0x0FFF_FFF8 {
                // Расширяем каталог на один кластер.
                let new_c = self.allocate_chain(1).ok_or(FatError::NoSpace)?;
                let zeros = vec![0u8; csize];
                self.write_cluster(new_c, &zeros);
                self.fat_set(cluster, new_c);
                let lba = self.cluster_to_lba(new_c);
                return Ok((lba, 0));
            }
            cluster = next;
        }
        Err(FatError::NoSpace)
    }

    // ---------- Запись файла ----------

    pub fn write_file(&mut self, entry: &mut FatEntry, data: &[u8]) -> FatResult<()> {
        if entry.first_cluster >= 2 {
            self.free_chain(entry.first_cluster);
        }

        if data.is_empty() {
            entry.first_cluster = 0;
            entry.size = 0;
            self.update_dir_entry(entry)?;
            return Ok(());
        }

        let csize = self.cluster_size_bytes();
        let need = (data.len() as u32 + csize - 1) / csize;
        let first = self.allocate_chain(need).ok_or(FatError::NoSpace)?;

        let mut cluster = first;
        let mut written = 0usize;
        let mut guard = 0u32;
        while cluster >= 2 && cluster < 0x0FFF_FFF8 && written < data.len() && guard < 1_000_000
        {
            guard += 1;
            let mut chunk = vec![0u8; csize as usize];
            let take = (data.len() - written).min(csize as usize);
            chunk[..take].copy_from_slice(&data[written..written + take]);
            self.write_cluster(cluster, &chunk);
            written += take;
            let next = self.fat_next(cluster);
            if next == cluster || next >= 0x0FFF_FFF8 {
                break;
            }
            cluster = next;
        }

        entry.first_cluster = first;
        entry.size = data.len() as u32;
        self.update_dir_entry(entry)?;
        Ok(())
    }

    fn update_dir_entry(&mut self, entry: &FatEntry) -> FatResult<()> {
        let lba = entry.dir_offset;
        let off = entry.entry_off;
        let mut sector = self.drive.read_sector(lba).ok_or(FatError::Io)?;

        let size = entry.size.to_le_bytes();
        sector[off + 28] = size[0];
        sector[off + 29] = size[1];
        sector[off + 30] = size[2];
        sector[off + 31] = size[3];

        let lo = (entry.first_cluster & 0xFFFF) as u16;
        let hi = ((entry.first_cluster >> 16) & 0xFFFF) as u16;
        sector[off + 20] = (hi & 0xFF) as u8;
        sector[off + 21] = (hi >> 8) as u8;
        sector[off + 26] = (lo & 0xFF) as u8;
        sector[off + 27] = (lo >> 8) as u8;

        if !self.drive.write_sector(lba, &sector) {
            return Err(FatError::Io);
        }
        Ok(())
    }

    // ---------- Удаление ----------

    pub fn remove(&mut self, entry: &FatEntry) -> FatResult<()> {
        if entry.kind == FatKind::Directory {
            // Проверяем, что каталог пуст.
            let raw = self.read_chain(entry.first_cluster, None);
            let mut i = 0usize;
            let mut has_content = false;
            while i + 32 <= raw.len() {
                let e = &raw[i..i + 32];
                i += 32;
                if e[0] == 0x00 {
                    break;
                }
                if e[0] == 0xE5 {
                    continue;
                }
                let attr = e[11];
                if attr == ATTR_LFN {
                    continue;
                }
                if attr & ATTR_VOLUME_ID != 0 {
                    continue;
                }
                if e[0] == b'.' {
                    continue;
                }
                has_content = true;
                break;
            }
            if has_content {
                return Err(FatError::NotADirectory);
            }
        }

        if entry.first_cluster >= 2 {
            self.free_chain(entry.first_cluster);
        }

        // Помечаем запись удалённой.
        let lba = entry.dir_offset;
        let off = entry.entry_off;
        let mut sector = self.drive.read_sector(lba).ok_or(FatError::Io)?;
        sector[off] = 0xE5;
        if !self.drive.write_sector(lba, &sector) {
            return Err(FatError::Io);
        }
        Ok(())
    }

    // ---------- Переименование ----------

    pub fn rename(&mut self, entry: &FatEntry, new_name: &str) -> FatResult<()> {
        if !valid_name(new_name) {
            return Err(FatError::InvalidName);
        }

        let lba = entry.dir_offset;
        let off = entry.entry_off;
        let mut sector = self.drive.read_sector(lba).ok_or(FatError::Io)?;
        let (base, ext) = to_short_name(new_name);
        for i in 0..8 {
            sector[off + i] = base[i];
        }
        for i in 0..3 {
            sector[off + 8 + i] = ext[i];
        }
        if !self.drive.write_sector(lba, &sector) {
            return Err(FatError::Io);
        }
        Ok(())
    }

    // ---------- Создание каталога ----------

    pub fn mkdir(&mut self, parent_cluster: u32, name: &str) -> FatResult<FatEntry> {
        if !valid_name(name) {
            return Err(FatError::InvalidName);
        }
        if self.find_in_dir(parent_cluster, name).is_some() {
            return Err(FatError::AlreadyExists);
        }

        let new_c = self.allocate_chain(1).ok_or(FatError::NoSpace)?;

        let mut buf = vec![0u8; self.cluster_size_bytes() as usize];
        write_dot_entries(&mut buf, new_c, parent_cluster);
        self.write_cluster(new_c, &buf);
        self.fat_set(new_c, FAT_EOC);

        let (lba, off) = self.find_free_dir_slot(parent_cluster)?;
        let mut sector = self.drive.read_sector(lba).ok_or(FatError::Io)?;

        let (base, ext) = to_short_name(name);
        let mut e = [0u8; 32];
        e[0..8].copy_from_slice(&base);
        e[8..11].copy_from_slice(&ext);
        e[11] = ATTR_DIRECTORY;

        let lo = (new_c & 0xFFFF) as u16;
        let hi = ((new_c >> 16) & 0xFFFF) as u16;
        e[20] = (hi & 0xFF) as u8;
        e[21] = (hi >> 8) as u8;
        e[26] = (lo & 0xFF) as u8;
        e[27] = (lo >> 8) as u8;

        for i in 0..32 {
            sector[off + i] = e[i];
        }
        if !self.drive.write_sector(lba, &sector) {
            return Err(FatError::Io);
        }

        Ok(FatEntry {
            name: name.to_string(),
            kind: FatKind::Directory,
            size: 0,
            first_cluster: new_c,
            dir_offset: lba,
            entry_off: off,
        })
    }

    // ---------- Flush ----------

    pub fn flush(&mut self) -> FatResult<()> {
        let _ = self.fs_info_sector;
        Ok(())
    }
}

// ---------- Утилиты ----------

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('/')
        && !name.contains('\\')
        && name != "."
        && name != ".."
        && name.len() <= 12
}

fn to_short_name(name: &str) -> ([u8; 8], [u8; 3]) {
    let mut base = [b' '; 8];
    let mut ext = [b' '; 3];
    let (stem, extension) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i + 1..]),
        _ => (name, ""),
    };
    for (i, b) in stem.bytes().take(8).enumerate() {
        base[i] = b.to_ascii_uppercase();
    }
    for (i, b) in extension.bytes().take(3).enumerate() {
        ext[i] = b.to_ascii_uppercase();
    }
    (base, ext)
}

fn parse_short_name(raw: &[u8]) -> String {
    let mut name = String::new();
    for &b in &raw[0..8] {
        if b == b' ' || b == 0 {
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

fn write_dot_entries(buf: &mut [u8], self_cluster: u32, parent_cluster: u32) {
    // "."
    let mut e = [0u8; 32];
    e[0..8].copy_from_slice(b".       ");
    e[8..11].copy_from_slice(b"   ");
    e[11] = ATTR_DIRECTORY;
    let lo = (self_cluster & 0xFFFF) as u16;
    let hi = ((self_cluster >> 16) & 0xFFFF) as u16;
    e[20] = (hi & 0xFF) as u8;
    e[21] = (hi >> 8) as u8;
    e[26] = (lo & 0xFF) as u8;
    e[27] = (lo >> 8) as u8;
    buf[0..32].copy_from_slice(&e);

    // ".."
    let mut e = [0u8; 32];
    e[0..8].copy_from_slice(b"..      ");
    e[8..11].copy_from_slice(b"   ");
    e[11] = ATTR_DIRECTORY;
    let lo = (parent_cluster & 0xFFFF) as u16;
    let hi = ((parent_cluster >> 16) & 0xFFFF) as u16;
    e[20] = (hi & 0xFF) as u8;
    e[21] = (hi >> 8) as u8;
    e[26] = (lo & 0xFF) as u8;
    e[27] = (lo >> 8) as u8;
    buf[32..64].copy_from_slice(&e);
}