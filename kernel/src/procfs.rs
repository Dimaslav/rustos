//! Виртуальная файловая система /proc.
//!
//! Не хранит данных — каждый раз генерирует содержимое на лету.
//! Поддерживаемые пути:
//! - `/proc`                — каталог
//! - `/proc/version`        — версия ядра
//! - `/proc/uptime`         — секунды с момента запуска
//! - `/proc/meminfo`        — состояние heap-аллокатора
//! - `/proc/ps`             — список потоков
//! - `/proc/self`           — каталог (псевдоним текущего процесса)
//! - `/proc/self/status`    — статус текущего потока
//! - `/proc/self/maps`      — карта адресного пространства
//!
//! Доступ идёт через VFS (`vfs::read`, `vfs::list`, `vfs::ramfs_stat`):
//! те функции сначала проверяют `is_proc_path()` и делегируют сюда.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

pub const ROOT: &str = "/proc";

pub fn is_proc_path(path: &str) -> bool {
    path == ROOT || path.starts_with("/proc/")
}

/// Прочитать виртуальный файл. `None` — путь не существует.
pub fn read(path: &str) -> Option<Vec<u8>> {
    let s: String = match path {
        "/proc/version"        => version_content(),
        "/proc/uptime"         => uptime_content(),
        "/proc/meminfo"        => meminfo_content(),
        "/proc/ps"             => ps_content(),
        "/proc/self/status"    => self_status_content(),
        "/proc/self/maps"      => self_maps_content(),
        _ => return None,
    };
    Some(s.into_bytes())
}

/// Существует ли путь.
pub fn exists(path: &str) -> bool {
    matches!(
        path,
        "/proc"
            | "/proc/"
            | "/proc/version"
            | "/proc/uptime"
            | "/proc/meminfo"
            | "/proc/ps"
            | "/proc/self"
            | "/proc/self/"
            | "/proc/self/status"
            | "/proc/self/maps"
    )
}

/// `(size, is_dir)`.
pub fn stat(path: &str) -> Option<(u64, bool)> {
    match path {
        "/proc" | "/proc/" | "/proc/self" | "/proc/self/" => Some((0, true)),
        _ => read(path).map(|data| (data.len() as u64, false)),
    }
}

/// Листинг каталога. Возвращает `(name, is_dir, size)`.
pub fn list(path: &str) -> Vec<(String, bool, u32)> {
    match path {
        "/proc" | "/proc/" => {
            vec![
                ("version".to_string(), false, version_content().len() as u32),
                ("uptime".to_string(), false, uptime_content().len() as u32),
                ("meminfo".to_string(), false, meminfo_content().len() as u32),
                ("ps".to_string(), false, ps_content().len() as u32),
                ("self".to_string(), true, 0),
            ]
        }
        "/proc/self" | "/proc/self/" => {
            vec![
                ("status".to_string(), false, self_status_content().len() as u32),
                ("maps".to_string(), false, self_maps_content().len() as u32),
            ]
        }
        _ => Vec::new(),
    }
}

// ---------- Содержимое файлов ----------

fn version_content() -> String {
    format!(
        "Rust OS 0.9.0\nbuilt with nightly rustc\ntarget: no_std x86_64\n"
    )
}

fn uptime_content() -> String {
    let t = crate::interrupts::ticks();
    let secs = t / 18;
    let frac = (t % 18) * 100 / 18;
    format!("{}.{:02}\n", secs, frac)
}

fn meminfo_content() -> String {
    use core::sync::atomic::Ordering;
    let (total, used, free) = {
        let a = crate::allocator::ALLOCATOR.lock();
        (crate::allocator::HEAP_SIZE, a.used(), a.free())
    };
    let phys_off = crate::memory::PHYS_OFFSET.load(Ordering::Acquire);
    format!(
        "HeapTotal:   {} bytes\nHeapUsed:    {} bytes\nHeapFree:    {} bytes\nPhysOffset:  {:#x}\n",
        total, used, free, phys_off
    )
}

fn ps_content() -> String {
    let threads = crate::sched::list_threads_snapshot();
    let mut s = String::from("PID   STATE      NAME\n");
    s.push_str("----  ---------  ----\n");
    for t in &threads {
        let state = match t.state {
            0 => "ready",
            1 => "running",
            2 => "sleeping",
            3 => "finished",
            _ => "?",
        };
        let end = t.name.iter().position(|&b| b == 0).unwrap_or(32);
        let name = core::str::from_utf8(&t.name[..end]).unwrap_or("?");
        s.push_str(&format!("{:<4}  {:<9}  {}\n", t.id, state, name));
    }
    s
}

fn self_status_content() -> String {
    let me = crate::sched::current_id();
    let ppid = crate::sched::parent_of(me).unwrap_or(0);
    let name = crate::sched::current_name();
    format!(
        "Pid:    {}\nPPid:   {}\nName:   {}\nState:  running\n",
        me, ppid, name
    )
}

/// Карта памяти текущего AS в формате, похожем на `/proc/<pid>/maps` Linux:
/// `start-end perms`
fn self_maps_content() -> String {
    let maps = unsafe { crate::memory::user_mappings_snapshot() };
    let mut s = String::new();
    if maps.is_empty() {
        s.push_str("(no user mappings — kernel address space)\n");
        return s;
    }
    for m in &maps {
        let perms = m.perms();
        let perms_str = core::str::from_utf8(&perms).unwrap_or("----");
        s.push_str(&format!(
            "{:08x}-{:08x} {}\n",
            m.start, m.end, perms_str
        ));
    }
    s
}