//! Кэш изображений. Загружает PNG/BMP из VFS, хранит в памяти, отдаёт Arc.

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use spin::Mutex;

use crate::bmp;
use crate::png;

pub struct CachedImage {
    pub name: String,
    pub width: usize,
    pub height: usize,
    /// RGB, top-down.
    pub pixels: Vec<u8>,
}

static CACHE: Mutex<Vec<Arc<CachedImage>>> = Mutex::new(Vec::new());

/// Загружает изображение из VFS по имени файла. Кэширует.
pub fn load(name: &str) -> Option<Arc<CachedImage>> {
    {
        let c = CACHE.lock();
        for img in c.iter() {
            if img.name == name {
                return Some(img.clone());
            }
        }
    }
    let data = crate::vfs::read(name).ok()?;
    let img = decode(name, &data)?;
    let arc = Arc::new(img);
    CACHE.lock().push(arc.clone());
    Some(arc)
}

/// Пробует имена по очереди, возвращает первое загруженное.
pub fn try_names(names: &[&str]) -> Option<Arc<CachedImage>> {
    for n in names {
        if let Some(img) = load(n) {
            return Some(img);
        }
    }
    None
}

/// Очистить кэш (например, при смене обоев).
pub fn clear_cache() {
    CACHE.lock().clear();
}

fn decode(name: &str, data: &[u8]) -> Option<CachedImage> {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".png") {
        let p = png::decode(data)?;
        return Some(CachedImage {
            name: name.into(),
            width: p.width,
            height: p.height,
            pixels: p.pixels,
        });
    }
    if lower.ends_with(".bmp") {
        let b = bmp::decode(data)?;
        return Some(CachedImage {
            name: name.into(),
            width: b.width,
            height: b.height,
            pixels: b.pixels,
        });
    }
    None
}