//! Минимальная реализация FAT32: монтирование, листинг каталогов,
//! чтение и запись файлов, создание/удаление каталогов.
//!
//! Поддерживается чтение и запись Long File Names (LFN):
//! длинные имена и кириллица сохраняются как LFN-записи (UTF-16),
//! рядом создаётся synthetic SHORT 8.3 с суффиксом `~N`.

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
    /// LBA сектора, где начинается short-entry.
    pub dir_offset: u32,
    /// Смещение внутри сектора для short-entry.
    pub entry_off: usize,
    /// Сколько LFN-записей стоит **перед** short-entry (0 — только short).
    pub lfn_count: usize,
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

            let lfn_count = lfn_parts.len();
            let name = if lfn_parts.is_empty() {
                short
            } else {
                lfn_parts.reverse();
                let mut s = String::new();
                for p in &lfn_parts {
                    s.push_str(p);
                }
                lfn_parts.clear();
                if s.is_empty() { short } else { s }
            };

            out.push(FatEntry {
                name,
                kind,
                size,
                first_cluster,
                dir_offset: 0,
                entry_off: 0,
                lfn_count,
            });
        }
        out
    }

    fn annotate_positions(&mut self, start_cluster: u32, entries: &mut [FatEntry]) {
        let raw = self.read_chain(start_cluster, None);

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

            let real_off = i - lfn_count * 32;
            let sector_in_chain = real_off / SECTOR_SIZE;
            let off_in_sector = real_off % SECTOR_SIZE;

            if entry_idx < entries.len() {
                if let Some(&lba) = sector_lbas.get(sector_in_chain) {
                    entries[entry_idx].dir_offset = lba;
                    entries[entry_idx].entry_off = off_in_sector;
                    entries[entry_idx].lfn_count = lfn_count;
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

        let (short_name, lfn_entries) = self.build_short_and_lfn(dir_cluster, name)?;
        let total_slots = lfn_entries.len() + 1;

        let (start_lba, start_off, _cross) =
            self.find_free_dir_slots(dir_cluster, total_slots)?;

        let mut all_entries: Vec<[u8; 32]> = Vec::with_capacity(total_slots);
        for lfn in &lfn_entries {
            all_entries.push(*lfn);
        }
        let mut short_entry = [0u8; 32];
        short_entry.copy_from_slice(&short_name);
        short_entry[11] = ATTR_ARCHIVE;
        all_entries.push(short_entry);

        self.write_dir_entries(dir_cluster, start_lba, start_off, &all_entries)?;

        Ok(FatEntry {
            name: name.to_string(),
            kind: FatKind::File,
            size: 0,
            first_cluster: 0,
            dir_offset: start_lba,
            entry_off: start_off + lfn_entries.len() * 32,
            lfn_count: lfn_entries.len(),
        })
    }

    fn write_dir_entries(
        &mut self,
        dir_cluster: u32,
        start_lba: u32,
        start_off: usize,
        entries: &[[u8; 32]],
    ) -> FatResult<()> {
        let spc = self.sectors_per_cluster as u32;
        let mut cluster = dir_cluster;
        let mut chain_lbas: Vec<u32> = Vec::new();
        let mut guard = 0u32;
        while cluster >= 2 && cluster < 0x0FFF_FFF8 && guard < 1_000_000 {
            guard += 1;
            let lba0 = self.cluster_to_lba(cluster);
            for s in 0..spc {
                chain_lbas.push(lba0 + s);
            }
            let next = self.fat_next(cluster);
            if next == cluster || next >= 0x0FFF_FFF8 {
                break;
            }
            cluster = next;
        }

        let mut idx = match chain_lbas.iter().position(|&l| l == start_lba) {
            Some(i) => i,
            None => return Err(FatError::Io),
        };
        let mut off = start_off;

        for entry in entries {
            let mut sector = self.drive.read_sector(chain_lbas[idx]).ok_or(FatError::Io)?;
            for k in 0..32 {
                sector[off + k] = entry[k];
            }
            self.drive.write_sector(chain_lbas[idx], &sector);

            off += 32;
            if off + 32 > SECTOR_SIZE {
                off = 0;
                idx += 1;
                if idx >= chain_lbas.len() {
                    return Err(FatError::NoSpace);
                }
            }
        }

        Ok(())
    }

    fn find_free_dir_slots(
        &mut self,
        dir_cluster: u32,
        n: usize,
    ) -> FatResult<(u32, usize, Option<(u32, usize)>)> {
        let csize = self.cluster_size_bytes() as usize;
        let entries_per_cluster = csize / 32;
        let mut cluster = dir_cluster;
        let mut guard = 0u32;

        while cluster >= 2 && cluster < 0x0FFF_FFF8 && guard < 1_000_000 {
            guard += 1;
            let lba = self.cluster_to_lba(cluster);

            let mut run_start: Option<usize> = None;
            let mut run_len = 0usize;
            for k in 0..entries_per_cluster {
                let off_in_cluster = k * 32;
                let sector_idx = off_in_cluster / SECTOR_SIZE;
                let off_in_sector = off_in_cluster % SECTOR_SIZE;
                let abs_lba = lba + sector_idx as u32;
                let sector = self.drive.read_sector(abs_lba).ok_or(FatError::Io)?;
                let first = sector[off_in_sector];

                if first == 0x00 || first == 0xE5 {
                    if run_start.is_none() {
                        run_start = Some(k);
                    }
                    run_len += 1;
                    if run_len >= n {
                        let s = run_start.unwrap();
                        let abs_off_in_cluster = s * 32;
                        let start_sector = lba + (abs_off_in_cluster / SECTOR_SIZE) as u32;
                        let start_off = abs_off_in_cluster % SECTOR_SIZE;
                        return Ok((start_sector, start_off, None));
                    }
                } else {
                    run_start = None;
                    run_len = 0;
                }
            }

            let next = self.fat_next(cluster);
            if next == cluster || next >= 0x0FFF_FFF8 {
                let new_c = self.allocate_chain(1).ok_or(FatError::NoSpace)?;
                let zeros = vec![0u8; csize];
                self.write_cluster(new_c, &zeros);
                self.fat_set(cluster, new_c);
                let lba = self.cluster_to_lba(new_c);
                return Ok((lba, 0, None));
            }
            cluster = next;
        }
        Err(FatError::NoSpace)
    }

    fn build_short_and_lfn(
        &mut self,
        dir_cluster: u32,
        name: &str,
    ) -> FatResult<([u8; 32], Vec<[u8; 32]>)> {
        let plain = plain_8_3(name);
        let (short11, need_lfn) = match plain {
            Some(bytes) => (bytes, false),
            None => (self.synthetic_short(dir_cluster, name)?, true),
        };

        let mut short_entry = [0u8; 32];
        short_entry[0..11].copy_from_slice(&short11);

        let lfn_entries = if need_lfn {
            make_lfn_entries(&short11, name)
        } else {
            Vec::new()
        };

        Ok((short_entry, lfn_entries))
    }

    fn synthetic_short(
        &mut self,
        dir_cluster: u32,
        name: &str,
    ) -> FatResult<[u8; 11]> {
        let mut prefix: Vec<u8> = Vec::new();
        for c in name.chars() {
            if prefix.len() >= 6 {
                break;
            }
            if c.is_ascii_alphanumeric() {
                prefix.push(c.to_ascii_uppercase() as u8);
            }
        }
        if prefix.is_empty() {
            prefix.push(b'F');
        }

        let ext: [u8; 3] = if let Some(dot) = name.rfind('.') {
            let e = &name[dot + 1..];
            let mut arr = [b' '; 3];
            let mut n = 0;
            for c in e.chars().take(3) {
                if c.is_ascii_alphanumeric() {
                    arr[n] = c.to_ascii_uppercase() as u8;
                    n += 1;
                }
            }
            arr
        } else {
            [b' '; 3]
        };

        for i in 1..=99u32 {
            let tail = format!("~{}", i);
            let tail_b = tail.as_bytes();
            let mut base = [b' '; 8];

            let keep = 8 - tail_b.len();
            let take = prefix.len().min(keep);
            for k in 0..take {
                base[k] = prefix[k];
            }
            for k in 0..tail_b.len() {
                base[take + k] = tail_b[k];
            }

            let mut full = [b' '; 11];
            full[0..8].copy_from_slice(&base);
            full[8..11].copy_from_slice(&ext);

            if self.find_in_dir_short(dir_cluster, &full).is_none() {
                return Ok(full);
            }
        }

        Err(FatError::NoSpace)
    }

    fn find_in_dir_short(&mut self, dir_cluster: u32, short11: &[u8; 11]) -> Option<FatEntry> {
        let raw = self.read_chain(dir_cluster, None);
        let mut i = 0usize;
        while i + 32 <= raw.len() {
            let e = &raw[i..i + 32];
            i += 32;
            if e[0] == 0x00 { break; }
            if e[0] == 0xE5 { continue; }
            if e[11] == ATTR_LFN { continue; }
            if e[11] & ATTR_VOLUME_ID != 0 { continue; }
            if &e[0..11] == short11 {
                return Some(FatEntry {
                    name: String::new(),
                    kind: FatKind::File,
                    size: 0,
                    first_cluster: 0,
                    dir_offset: 0,
                    entry_off: 0,
                    lfn_count: 0,
                });
            }
        }
        None
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
            let raw = self.read_chain(entry.first_cluster, None);
            let mut i = 0usize;
            let mut has_content = false;
            while i + 32 <= raw.len() {
                let e = &raw[i..i + 32];
                i += 32;
                if e[0] == 0x00 { break; }
                if e[0] == 0xE5 { continue; }
                let attr = e[11];
                if attr == ATTR_LFN { continue; }
                if attr & ATTR_VOLUME_ID != 0 { continue; }
                if e[0] == b'.' { continue; }
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

        let total = entry.lfn_count + 1;
        self.mark_dir_entries_deleted(
            entry.dir_offset,
            entry.entry_off,
            entry.lfn_count,
            total,
        )?;
        Ok(())
    }

    fn mark_dir_entries_deleted(
        &mut self,
        lba: u32,
        off: usize,
        back: usize,
        total: usize,
    ) -> FatResult<()> {
        let mut cur_lba = lba;
        let mut cur_off = off;

        for _ in 0..back {
            if cur_off >= 32 {
                cur_off -= 32;
            } else {
                if cur_lba == 0 {
                    return Err(FatError::Io);
                }
                cur_lba -= 1;
                cur_off = SECTOR_SIZE - 32;
            }
        }

        for _ in 0..total {
            let mut sector = self.drive.read_sector(cur_lba).ok_or(FatError::Io)?;
            sector[cur_off] = 0xE5;
            self.drive.write_sector(cur_lba, &sector);

            cur_off += 32;
            if cur_off >= SECTOR_SIZE {
                cur_off = 0;
                cur_lba += 1;
            }
        }
        Ok(())
    }

    // ---------- Переименование ----------

    /// Переименовать `entry` в `new_name`.
    ///
    /// Порядок операций выбран так, чтобы при любой ошибке (нет места, I/O)
    /// исходная запись осталась целой:
    ///   1. сохранить метаданные старой записи (attr, first_cluster, size);
    ///   2. построить новые LFN + short;
    ///   3. найти свободные слоты, **не трогая** старую запись;
    ///   4. записать новые записи;
    ///   5. только после успеха — пометить старые как удалённые.
    ///
    /// Ограничение MVP: поиск коллизий и свободных слотов идёт в корне.
    pub fn rename(&mut self, entry: &FatEntry, new_name: &str) -> FatResult<()> {
        if !valid_name(new_name) {
            return Err(FatError::InvalidName);
        }

        if entry.name != new_name
            && self.find_in_dir(self.root_cluster, new_name).is_some()
        {
            return Err(FatError::AlreadyExists);
        }

        let old_lba = entry.dir_offset;
        let old_off = entry.entry_off;
        let old_lfn_count = entry.lfn_count;
        let first_cluster = entry.first_cluster;
        let size = entry.size;

        let old_sector = self.drive.read_sector(old_lba).ok_or(FatError::Io)?;
        if old_off + 32 > old_sector.len() {
            return Err(FatError::Io);
        }
        let attr = old_sector[old_off + 11];

        let dir_cluster = self.root_cluster;
        let (short_entry_template, lfn_entries) =
            self.build_short_and_lfn(dir_cluster, new_name)?;
        let total_new = lfn_entries.len() + 1;

        let (new_lba, new_off, _cross) =
            self.find_free_dir_slots(dir_cluster, total_new)?;

        let mut short_entry = short_entry_template;
        short_entry[11] = attr;
        let cl_lo = (first_cluster & 0xFFFF) as u16;
        let cl_hi = ((first_cluster >> 16) & 0xFFFF) as u16;
        short_entry[20] = (cl_hi & 0xFF) as u8;
        short_entry[21] = (cl_hi >> 8) as u8;
        short_entry[26] = (cl_lo & 0xFF) as u8;
        short_entry[27] = (cl_lo >> 8) as u8;
        let sz = size.to_le_bytes();
        short_entry[28] = sz[0];
        short_entry[29] = sz[1];
        short_entry[30] = sz[2];
        short_entry[31] = sz[3];

        let mut all_entries: Vec<[u8; 32]> = Vec::with_capacity(total_new);
        for lfn in &lfn_entries {
            all_entries.push(*lfn);
        }
        all_entries.push(short_entry);

        self.write_dir_entries(dir_cluster, new_lba, new_off, &all_entries)?;

        self.mark_dir_entries_deleted(
            old_lba,
            old_off,
            old_lfn_count,
            old_lfn_count + 1,
        )?;

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

        let (short_entry, lfn_entries) =
            self.build_short_and_lfn(parent_cluster, name)?;
        let total_slots = lfn_entries.len() + 1;

        let (start_lba, start_off, _cross) =
            self.find_free_dir_slots(parent_cluster, total_slots)?;

        let mut short_entry = short_entry;
        short_entry[11] = ATTR_DIRECTORY;
        let lo = (new_c & 0xFFFF) as u16;
        let hi = ((new_c >> 16) & 0xFFFF) as u16;
        short_entry[20] = (hi & 0xFF) as u8;
        short_entry[21] = (hi >> 8) as u8;
        short_entry[26] = (lo & 0xFF) as u8;
        short_entry[27] = (lo >> 8) as u8;

        let mut all_entries: Vec<[u8; 32]> = Vec::with_capacity(total_slots);
        for lfn in &lfn_entries {
            all_entries.push(*lfn);
        }
        all_entries.push(short_entry);

        self.write_dir_entries(parent_cluster, start_lba, start_off, &all_entries)?;

        Ok(FatEntry {
            name: name.to_string(),
            kind: FatKind::Directory,
            size: 0,
            first_cluster: new_c,
            dir_offset: start_lba,
            entry_off: start_off + lfn_entries.len() * 32,
            lfn_count: lfn_entries.len(),
        })
    }

    // ---------- Path-based helpers ----------

    /// Разобрать путь от корня, вернуть `FatEntry` последнего компонента.
    /// Корень (`""`, `"/"`, `"C:"`) → `None` (у корня нет FatEntry).
    ///
    /// Поддерживает только спуск, без `..` — см. ограничение MVP.
    pub fn resolve_path(&mut self, path: &str) -> Option<FatEntry> {
        let cleaned = path
            .trim_start_matches("C:")
            .trim_start_matches('/')
            .trim_end_matches('/');
        if cleaned.is_empty() {
            return None;
        }
        let parts: Vec<&str> = cleaned
            .split('/')
            .filter(|p| !p.is_empty() && *p != ".")
            .collect();
        if parts.is_empty() {
            return None;
        }

        let mut current_cluster = self.root_cluster;
        for (i, part) in parts.iter().enumerate() {
            if *part == ".." {
                return None;
            }
            let entry = self.find_in_dir(current_cluster, part)?;
            if i == parts.len() - 1 {
                return Some(entry);
            }
            if entry.kind != FatKind::Directory {
                return None;
            }
            current_cluster = entry.first_cluster;
        }
        None
    }

    /// Кластер родительского каталога для `path`:
    /// - `/foo/bar.txt` → кластер `foo`
    /// - `bar.txt`      → root_cluster
    /// - `""`, `"/"`    → root_cluster
    pub fn parent_cluster_of(&mut self, path: &str) -> Option<u32> {
        let cleaned = path
            .trim_start_matches("C:")
            .trim_start_matches('/')
            .trim_end_matches('/');
        if cleaned.is_empty() {
            return Some(self.root_cluster);
        }
        let parts: Vec<&str> = cleaned
            .split('/')
            .filter(|p| !p.is_empty() && *p != ".")
            .collect();
        if parts.len() <= 1 {
            return Some(self.root_cluster);
        }
        let parent_path = parts[..parts.len() - 1].join("/");
        let parent = self.resolve_path(&parent_path)?;
        if parent.kind != FatKind::Directory {
            return None;
        }
        Some(parent.first_cluster)
    }

    /// Листинг каталога по пути. `""`, `"/"`, `"C:"` → корень.
    pub fn list_dir_by_path(&mut self, path: &str) -> Vec<FatEntry> {
        let cleaned = path
            .trim_start_matches("C:")
            .trim_start_matches('/')
            .trim_end_matches('/');
        if cleaned.is_empty() {
            return self.list_root();
        }
        match self.resolve_path(cleaned) {
            Some(e) if e.kind == FatKind::Directory => self.list_dir(&e),
            _ => Vec::new(),
        }
    }

    /// Прочитать файл по пути.
    pub fn read_file_by_path(&mut self, path: &str) -> Option<Vec<u8>> {
        let entry = self.resolve_path(path)?;
        if entry.kind != FatKind::File {
            return None;
        }
        Some(self.read_file(&entry))
    }

    /// Записать файл по пути. Если файла нет — создать в родителе.
    pub fn write_file_by_path(&mut self, path: &str, data: &[u8]) -> bool {
        let cleaned = path
            .trim_start_matches("C:")
            .trim_start_matches('/')
            .trim_end_matches('/');
        if cleaned.is_empty() {
            return false;
        }
        let name = match cleaned.rsplit('/').next() {
            Some(n) if !n.is_empty() => n,
            _ => return false,
        };
        let parent = match self.parent_cluster_of(cleaned) {
            Some(p) => p,
            None => return false,
        };
        if let Some(mut e) = self.find_in_dir(parent, name) {
            if e.kind != FatKind::File {
                return false;
            }
            return self.write_file(&mut e, data).is_ok();
        }
        match self.create_file(parent, name) {
            Ok(mut e) => {
                let ok = self.write_file(&mut e, data).is_ok();
                let _ = self.flush();
                ok
            }
            Err(_) => false,
        }
    }

    /// `mkdir` по пути.
    pub fn mkdir_by_path(&mut self, path: &str) -> bool {
        let cleaned = path
            .trim_start_matches("C:")
            .trim_start_matches('/')
            .trim_end_matches('/');
        if cleaned.is_empty() {
            return false;
        }
        let name = match cleaned.rsplit('/').next() {
            Some(n) if !n.is_empty() => n,
            _ => return false,
        };
        let parent = match self.parent_cluster_of(cleaned) {
            Some(p) => p,
            None => return false,
        };
        self.mkdir(parent, name).is_ok()
    }

    /// Удалить файл/пустой каталог по пути.
    pub fn remove_by_path(&mut self, path: &str) -> bool {
        let entry = match self.resolve_path(path) {
            Some(e) => e,
            None => return false,
        };
        self.remove(&entry).is_ok()
    }

    /// Переименовать по пути.
    pub fn rename_by_path(&mut self, old: &str, new_name: &str) -> bool {
        let entry = match self.resolve_path(old) {
            Some(e) => e,
            None => return false,
        };
        self.rename(&entry, new_name).is_ok()
    }

    /// Метаданные по пути: `(size, is_dir)`.
    pub fn stat_by_path(&mut self, path: &str) -> Option<(u64, bool)> {
        let entry = self.resolve_path(path)?;
        Some((entry.size as u64, entry.kind == FatKind::Directory))
    }

    // ---------- Flush ----------

    pub fn flush(&mut self) -> FatResult<()> {
        let _ = self.fs_info_sector;
        Ok(())
    }
}

// ---------- Утилиты имён ----------

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('/')
        && !name.contains('\\')
        && name != "."
        && name != ".."
        && name.len() <= 255
}

fn plain_8_3(name: &str) -> Option<[u8; 11]> {
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 && i < name.len() - 1 => (&name[..i], &name[i + 1..]),
        Some(i) if i > 0 => (&name[..i], ""),
        _ => (name, ""),
    };

    if stem.is_empty() || stem.len() > 8 || ext.len() > 3 {
        return None;
    }
    for c in stem.chars().chain(ext.chars()) {
        if !c.is_ascii() { return None; }
        let u = c as u8;
        let ok = u.is_ascii_alphanumeric()
            || u == b'_'
            || u == b'-';
        if !ok { return None; }
    }

    let mut out = [b' '; 11];
    for (i, b) in stem.bytes().take(8).enumerate() {
        out[i] = b.to_ascii_uppercase();
    }
    for (i, b) in ext.bytes().take(3).enumerate() {
        out[8 + i] = b.to_ascii_uppercase();
    }
    Some(out)
}

fn parse_short_name(raw: &[u8]) -> String {
    let mut name = String::new();
    for &b in &raw[0..8] {
        if b == b' ' || b == 0 { break; }
        name.push((b as char).to_ascii_uppercase());
    }
    let mut ext = String::new();
    for &b in &raw[8..11] {
        if b == b' ' || b == 0 { break; }
        ext.push((b as char).to_ascii_lowercase());
    }
    if ext.is_empty() { name } else { format!("{}.{}", name, ext) }
}

fn lfn_checksum(short11: &[u8; 11]) -> u8 {
    let mut sum: u8 = 0;
    for &b in short11.iter() {
        sum = ((sum & 1) << 7).wrapping_add(sum >> 1).wrapping_add(b);
    }
    sum
}

fn make_lfn_entries(short11: &[u8; 11], long: &str) -> Vec<[u8; 32]> {
    let mut utf16: Vec<u16> = long.encode_utf16().collect();
    utf16.push(0);

    let mut chunks: Vec<[u16; 13]> = Vec::new();
    let mut idx = 0;
    while idx < utf16.len() {
        let mut c = [0xFFFFu16; 13];
        for k in 0..13 {
            if idx + k < utf16.len() {
                c[k] = utf16[idx + k];
            }
        }
        chunks.push(c);
        idx += 13;
    }

    let checksum = lfn_checksum(short11);
    let total = chunks.len();

    let offsets = [
        1usize, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30,
    ];

    let mut result: Vec<[u8; 32]> = Vec::with_capacity(total);
    for (i, chunk) in chunks.iter().enumerate() {
        let ord = (total - i) as u8;
        let is_last = i == 0;
        let mut e = [0u8; 32];
        e[0] = if is_last { ord | 0x40 } else { ord };
        e[11] = ATTR_LFN;
        e[12] = 0;
        e[13] = checksum;
        for (k, &off) in offsets.iter().enumerate() {
            let c = chunk[k];
            e[off] = (c & 0xFF) as u8;
            e[off + 1] = ((c >> 8) & 0xFF) as u8;
        }
        result.push(e);
    }
    result
}

fn write_dot_entries(buf: &mut [u8], self_cluster: u32, parent_cluster: u32) {
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