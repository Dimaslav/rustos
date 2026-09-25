//! Унифицированный VFS-слой.
//!
//! Один глобальный `Vfs` владеет всеми файловыми системами:
//! - RAMFS (in-memory дерево узлов) — mount на `/`
//! - FAT32 (диск, LBA28)             — mount на `C:`
//!
//! Все обращения идут через функции-обёртки, которые берут `VFS.lock()`
//! и роутят по префиксу пути.
//!
//! Ограничения MVP:
//! - FAT32 поддерживает только корневой каталог (без поддиректорий).
//!   Путь с `/` после `C:` вернёт NotFound.
//! - RAMFS поддерживает произвольную вложенность.

use alloc::string::String;
use alloc::vec::Vec;
use spin::Mutex;

use crate::fat32::FatKind;
use crate::fs::{FileSystem, NodeKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VfsError {
    NotFound,
    AlreadyExists,
    NotADirectory,
    InvalidName,
    NoDisk,
    Io,
    IsADirectory,
}

pub type VfsResult<T> = Result<T, VfsError>;

/// Метаданные одной записи в листинге.
#[derive(Clone, Debug)]
pub struct VfsEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u32,
}

pub struct Vfs {
    pub ramfs: FileSystem,
    pub fat32: Option<crate::fat32::Fat32>,
}

pub static VFS: Mutex<Option<Vfs>> = Mutex::new(None);

/// Инициализация. Вызывать один раз в `kernel_main` — до первого
/// syscall'а или обращения GUI к FS.
pub fn init(fat32: Option<crate::fat32::Fat32>) {
    *VFS.lock() = Some(Vfs {
        ramfs: FileSystem::new(),
        fat32,
    });
}

/// Роутинг по префиксу: `(is_fat32, относительный_путь)`.
fn split(path: &str) -> (bool, &str) {
    if let Some(rest) = path.strip_prefix("C:") {
        (true, rest.trim_start_matches('/'))
    } else {
        (false, path.trim_start_matches('/'))
    }
}

// ---------- RAMFS wrappers ----------

pub fn ramfs_list_meta(path: &str) -> Vec<(String, bool, usize)> {
    let g = VFS.lock();
    let v = match g.as_ref() {
        Some(v) => v,
        None => return Vec::new(),
    };
    v.ramfs
        .list(path)
        .into_iter()
        .map(|e| (e.name, e.kind == NodeKind::Directory, e.size))
        .collect()
}

pub fn ramfs_read(path: &str) -> Option<Vec<u8>> {
    let g = VFS.lock();
    let v = g.as_ref()?;
    v.ramfs.read(path)
}

pub fn ramfs_write(path: &str, data: &[u8]) -> bool {
    let mut g = VFS.lock();
    let v = match g.as_mut() {
        Some(v) => v,
        None => return false,
    };
    v.ramfs.write(path, data)
}

pub fn ramfs_create_file(parent: &str, name: &str, data: &[u8]) -> bool {
    let mut g = VFS.lock();
    let v = match g.as_mut() {
        Some(v) => v,
        None => return false,
    };
    v.ramfs.create_file(parent, name, data)
}

pub fn ramfs_mkdir(parent: &str, name: &str) -> bool {
    let mut g = VFS.lock();
    let v = match g.as_mut() {
        Some(v) => v,
        None => return false,
    };
    v.ramfs.mkdir(parent, name)
}

pub fn ramfs_remove(path: &str) -> bool {
    let mut g = VFS.lock();
    let v = match g.as_mut() {
        Some(v) => v,
        None => return false,
    };
    v.ramfs.remove(path)
}

pub fn ramfs_rename(path: &str, new_name: &str) -> bool {
    let mut g = VFS.lock();
    let v = match g.as_mut() {
        Some(v) => v,
        None => return false,
    };
    v.ramfs.rename(path, new_name)
}

pub fn ramfs_exists(path: &str) -> bool {
    let g = VFS.lock();
    match g.as_ref() {
        Some(v) => v.ramfs.exists(path),
        None => false,
    }
}

// ---------- FAT32 wrappers ----------

pub fn fat32_list_root() -> Vec<(String, bool, u32)> {
    let mut g = VFS.lock();
    let v = match g.as_mut() {
        Some(v) => v,
        None => return Vec::new(),
    };
    let fat = match v.fat32.as_mut() {
        Some(f) => f,
        None => return Vec::new(),
    };
    fat.list_root()
        .into_iter()
        .map(|e| (e.name, e.kind == FatKind::Directory, e.size))
        .collect()
}

pub fn fat32_read_file(name: &str) -> Option<Vec<u8>> {
    let mut g = VFS.lock();
    let v = g.as_mut()?;
    let fat = v.fat32.as_mut()?;
    let root = fat.root_cluster();
    let entry = fat.find_in_dir(root, name)?;
    Some(fat.read_file(&entry))
}

pub fn fat32_create_file(name: &str) -> bool {
    let mut g = VFS.lock();
    let v = match g.as_mut() {
        Some(v) => v,
        None => return false,
    };
    let fat = match v.fat32.as_mut() {
        Some(f) => f,
        None => return false,
    };
    let root = fat.root_cluster();
    fat.create_file(root, name).is_ok()
}

pub fn fat32_write_file(name: &str, data: &[u8]) -> bool {
    let mut g = VFS.lock();
    let v = match g.as_mut() {
        Some(v) => v,
        None => return false,
    };
    let fat = match v.fat32.as_mut() {
        Some(f) => f,
        None => return false,
    };
    let root = fat.root_cluster();
    if let Some(mut e) = fat.find_in_dir(root, name) {
        fat.write_file(&mut e, data).is_ok()
    } else {
        match fat.create_file(root, name) {
            Ok(mut e) => {
                let ok = fat.write_file(&mut e, data).is_ok();
                let _ = fat.flush();
                ok
            }
            Err(_) => false,
        }
    }
}

pub fn fat32_mkdir(name: &str) -> bool {
    let mut g = VFS.lock();
    let v = match g.as_mut() {
        Some(v) => v,
        None => return false,
    };
    let fat = match v.fat32.as_mut() {
        Some(f) => f,
        None => return false,
    };
    let root = fat.root_cluster();
    fat.mkdir(root, name).is_ok()
}

pub fn fat32_remove(name: &str) -> bool {
    let mut g = VFS.lock();
    let v = match g.as_mut() {
        Some(v) => v,
        None => return false,
    };
    let fat = match v.fat32.as_mut() {
        Some(f) => f,
        None => return false,
    };
    let root = fat.root_cluster();
    if let Some(e) = fat.find_in_dir(root, name) {
        fat.remove(&e).is_ok()
    } else {
        false
    }
}

pub fn fat32_rename(old_name: &str, new_name: &str) -> bool {
    let mut g = VFS.lock();
    let v = match g.as_mut() {
        Some(v) => v,
        None => return false,
    };
    let fat = match v.fat32.as_mut() {
        Some(f) => f,
        None => return false,
    };
    let root = fat.root_cluster();
    if let Some(e) = fat.find_in_dir(root, old_name) {
        fat.rename(&e, new_name).is_ok()
    } else {
        false
    }
}

pub fn fat32_exists(name: &str) -> bool {
    let mut g = VFS.lock();
    let v = match g.as_mut() {
        Some(v) => v,
        None => return false,
    };
    let fat = match v.fat32.as_mut() {
        Some(f) => f,
        None => return false,
    };
    let root = fat.root_cluster();
    fat.find_in_dir(root, name).is_some()
}

// ---------- Единый API ----------

pub fn list(path: &str) -> VfsResult<Vec<VfsEntry>> {
    let (is_fat, _rel) = split(path);
    if is_fat {
        Ok(fat32_list_root()
            .into_iter()
            .map(|(n, d, s)| VfsEntry {
                name: n,
                is_dir: d,
                size: s,
            })
            .collect())
    } else {
        Ok(ramfs_list_meta(path)
            .into_iter()
            .map(|(n, d, s)| VfsEntry {
                name: n,
                is_dir: d,
                size: s as u32,
            })
            .collect())
    }
}

pub fn read(path: &str) -> VfsResult<Vec<u8>> {
    let (is_fat, rel) = split(path);
    if is_fat {
        fat32_read_file(rel).ok_or(VfsError::NotFound)
    } else {
        ramfs_read(path).ok_or(VfsError::NotFound)
    }
}

pub fn write(path: &str, data: &[u8]) -> VfsResult<()> {
    let (is_fat, rel) = split(path);
    let ok = if is_fat {
        fat32_write_file(rel, data)
    } else {
        ramfs_write(path, data)
    };
    if ok {
        Ok(())
    } else {
        Err(VfsError::Io)
    }
}

pub fn exists(path: &str) -> bool {
    let (is_fat, rel) = split(path);
    if is_fat {
        fat32_exists(rel)
    } else {
        ramfs_exists(path)
    }
}