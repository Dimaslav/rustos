//! Per-process file descriptor table.

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use spin::Mutex;

use crate::pipe;
use crate::sched::ThreadId;

/// Состояние открытого на запись файла. Разделяется между всеми fd,
/// полученными через `dup2`, чтобы не было двойного flush.
pub struct FileWriteState {
    pub name: String,
    pub append: bool,
    /// Накопленные данные с момента открытия.
    pub data: Vec<u8>,
    /// Сколько байт из `data` уже записано на диск.
    pub flushed_len: usize,
}

#[derive(Clone)]
pub enum FdKind {
    Stdin,
    Stdout,
    Stderr,
    File {
        name: String,
        data: Vec<u8>,
        pos: usize,
    },
    FileWrite {
        state: Arc<Mutex<FileWriteState>>,
    },
    PipeRead { pipe_id: u32 },
    PipeWrite { pipe_id: u32 },
}

pub struct FdEntry {
    pub pid: ThreadId,
    pub fd: u32,
    pub kind: FdKind,
}

pub static FD_TABLE: Mutex<Vec<FdEntry>> = Mutex::new(Vec::new());

/// Начальная инициализация (для PID, у которого нет родителя).
pub fn init_for_pid(pid: ThreadId) {
    let mut t = FD_TABLE.lock();
    t.retain(|e| e.pid != pid);
    t.push(FdEntry { pid, fd: 0, kind: FdKind::Stdin });
    t.push(FdEntry { pid, fd: 1, kind: FdKind::Stdout });
    t.push(FdEntry { pid, fd: 2, kind: FdKind::Stderr });
}

/// Копирует fd-таблицу родителя в нового процесса.
pub fn fork_from(parent: ThreadId, child: ThreadId) {
    let mut cloned: Vec<FdKind> = {
        let t = FD_TABLE.lock();
        t.iter().filter(|e| e.pid == parent).map(|e| e.kind.clone()).collect()
    };
    if cloned.is_empty() {
        init_for_pid(child);
        return;
    }
    // Инкрементим счётчики pipe для клонированных концов.
    for k in &cloned {
        match k {
            FdKind::PipeRead { pipe_id } => { pipe::add_reader(*pipe_id); }
            FdKind::PipeWrite { pipe_id } => { pipe::add_writer(*pipe_id); }
            _ => {}
        }
    }
    let mut t = FD_TABLE.lock();
    t.retain(|e| e.pid != child);
    for (i, k) in cloned.drain(..).enumerate() {
        t.push(FdEntry { pid: child, fd: i as u32, kind: k });
    }
    let has0 = t.iter().any(|e| e.pid == child && e.fd == 0);
    let has1 = t.iter().any(|e| e.pid == child && e.fd == 1);
    let has2 = t.iter().any(|e| e.pid == child && e.fd == 2);
    if !has0 { t.push(FdEntry { pid: child, fd: 0, kind: FdKind::Stdin }); }
    if !has1 { t.push(FdEntry { pid: child, fd: 1, kind: FdKind::Stdout }); }
    if !has2 { t.push(FdEntry { pid: child, fd: 2, kind: FdKind::Stderr }); }
}

pub fn alloc_fd(pid: ThreadId, kind: FdKind) -> u32 {
    let mut t = FD_TABLE.lock();
    let mut fd = 3u32;
    loop {
        if !t.iter().any(|e| e.pid == pid && e.fd == fd) {
            t.push(FdEntry { pid, fd, kind });
            return fd;
        }
        fd += 1;
    }
}

pub fn get(pid: ThreadId, fd: u32) -> Option<FdKind> {
    let t = FD_TABLE.lock();
    t.iter().find(|e| e.pid == pid && e.fd == fd).map(|e| e.kind.clone())
}

/// Копирует `old_fd` в `new_fd`. Старое значение `new_fd` закрывается.
pub fn dup2(pid: ThreadId, old_fd: u32, new_fd: u32) -> bool {
    let kind = match get(pid, old_fd) {
        Some(k) => k,
        None => return false,
    };

    match &kind {
        FdKind::PipeRead { pipe_id } => { pipe::add_reader(*pipe_id); }
        FdKind::PipeWrite { pipe_id } => { pipe::add_writer(*pipe_id); }
        _ => {}
    }

    set(pid, new_fd, kind);
    true
}

pub fn set(pid: ThreadId, fd: u32, kind: FdKind) {
    let old = {
        let mut t = FD_TABLE.lock();
        if let Some(idx) = t.iter().position(|e| e.pid == pid && e.fd == fd) {
            let old = t[idx].kind.clone();
            t[idx].kind = kind;
            Some(old)
        } else {
            t.push(FdEntry { pid, fd, kind });
            None
        }
    };
    if let Some(k) = old {
        close_kind(k);
    }
}

pub fn close(pid: ThreadId, fd: u32) -> bool {
    let kind = {
        let mut t = FD_TABLE.lock();
        if let Some(idx) = t.iter().position(|e| e.pid == pid && e.fd == fd) {
            let kind = t[idx].kind.clone();
            t.remove(idx);
            Some(kind)
        } else {
            None
        }
    };
    if let Some(k) = kind {
        close_kind(k);
        true
    } else {
        false
    }
}

pub fn update_file_pos(pid: ThreadId, fd: u32, new_pos: usize) {
    let mut t = FD_TABLE.lock();
    if let Some(e) = t.iter_mut().find(|e| e.pid == pid && e.fd == fd) {
        if let FdKind::File { pos, .. } = &mut e.kind {
            *pos = new_pos;
        }
    }
}

fn close_kind(kind: FdKind) {
    match kind {
        FdKind::PipeRead { pipe_id } => pipe::close_reader(pipe_id),
        FdKind::PipeWrite { pipe_id } => pipe::close_writer(pipe_id),
        FdKind::FileWrite { state } => flush_file_write(&state),
        _ => {}
    }
}

pub fn close_all_for_pid(pid: ThreadId) {
    let to_close: Vec<FdKind> = {
        let mut t = FD_TABLE.lock();
        let kinds = t.iter().filter(|e| e.pid == pid).map(|e| e.kind.clone()).collect();
        t.retain(|e| e.pid != pid);
        kinds
    };
    for k in to_close {
        close_kind(k);
    }
}

// ---------- FileWrite helpers ----------

/// Создать новый `FileWrite`. При `append=false` — немедленно обнулить файл.
pub fn open_write(pid: ThreadId, path: String, append: bool) -> u32 {
    if !append {
        // Truncate/create.
        write_or_create(&path, &[]);
    }
    let state = Arc::new(Mutex::new(FileWriteState {
        name: path,
        append,
        data: Vec::new(),
        flushed_len: 0,
    }));
    alloc_fd(pid, FdKind::FileWrite { state })
}

/// Дописать байты в буфер `FileWrite`. Возвращает число байт.
pub fn write_file(state: &Arc<Mutex<FileWriteState>>, bytes: &[u8]) -> usize {
    let mut s = state.lock();
    s.data.extend_from_slice(bytes);
    bytes.len()
}

fn flush_file_write(state: &Arc<Mutex<FileWriteState>>) {
    let (name, append, full, new_part) = {
        let mut s = state.lock();
        if s.flushed_len >= s.data.len() {
            return;
        }
        let result = (
            s.name.clone(),
            s.append,
            s.data.clone(),
            s.data[s.flushed_len..].to_vec(),
        );
        s.flushed_len = s.data.len();
        result
    };

    if append {
        let existing = crate::vfs::read(&name).unwrap_or_default();
        let mut v = existing;
        v.extend_from_slice(&new_part);
        write_or_create(&name, &v);
    } else {
        write_or_create(&name, &full);
    }
}

/// Записать файл, создавая его при необходимости.
pub fn write_or_create(path: &str, data: &[u8]) {
    if crate::vfs::write(path, data).is_ok() {
        return;
    }
    // RAMFS: файла нет — создаём.
    if !path.starts_with("C:") {
        let parent = crate::fs::parent_path(path);
        let name = path.rsplit('/').next().unwrap_or(path);
        let _ = crate::vfs::ramfs_create_file(&parent, name, data);
    }
}