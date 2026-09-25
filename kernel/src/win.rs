//! User-owned окна: shared framebuffer + event queue.

use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use spin::Mutex;

/// Высота заголовка окна (совпадает с `draw_foreign`).
pub const TITLE_BAR_H: i32 = 26;

pub const WIN_BUF_VADDR: u64 = 0x0100_0000;
pub const MAX_WIN_W: u32 = 800;
pub const MAX_WIN_H: u32 = 600;

#[derive(Clone, Copy)]
#[repr(C)]
pub struct WinEvent {
    pub kind: u32,
    pub x: i32,
    pub y: i32,
    pub code: u32,
}

pub const EV_MOUSE_DOWN: u32 = 0;
pub const EV_MOUSE_UP: u32 = 1;
pub const EV_MOUSE_MOVE: u32 = 2;
pub const EV_CLOSE: u32 = 3;

pub struct ForeignWindow {
    pub id: u64,
    pub pid: u64,
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub title: String,
    pub user_buf: u64,
    pub buf_len: usize,
    pub shadow: Vec<u8>,
    pub dirty: bool,
    pub events: VecDeque<WinEvent>,
    pub alive: bool,
}

static FOREIGN: Mutex<Vec<ForeignWindow>> = Mutex::new(Vec::new());
static NEXT_ID: Mutex<u64> = Mutex::new(1);

pub fn alloc_window(
    pid: u64,
    w: u32,
    h: u32,
    title: String,
    user_buf: u64,
) -> Option<u64> {
    if w == 0 || h == 0 || w > MAX_WIN_W || h > MAX_WIN_H {
        return None;
    }
    let id = { let mut n = NEXT_ID.lock(); let v = *n; *n += 1; v };
    let buf_len = (w as usize) * (h as usize) * 4;
    let fw = ForeignWindow {
        id,
        pid,
        x: 100 + ((id as i32) * 40) % 400,
        y: 100 + ((id as i32) * 30) % 300,
        w,
        h,
        title,
        user_buf,
        buf_len,
        shadow: vec![0u8; buf_len],
        dirty: true,
        events: VecDeque::new(),
        alive: true,
    };
    FOREIGN.lock().push(fw);
    Some(id)
}

/// Копирует user-буфер в kernel-shadow. CR3 в syscall = user-AS, поэтому
/// по `user_buf` можно читать напрямую.
///
/// Возвращает `false`, если окно исчезло или lock занят (main сейчас
/// перерисовывает — тогда пропускаем кадр, следующий пройдёт).
pub fn present(id: u64) -> bool {
    // try_lock: избегаем busy-wait, если main рисует прямо сейчас.
    let mut guard = match FOREIGN.try_lock() {
        Some(g) => g,
        None => return false,
    };
    let Some(fw) = guard.iter_mut().find(|f| f.id == id) else {
        return false;
    };
    unsafe {
        let src = fw.user_buf as *const u8;
        for i in 0..fw.buf_len {
            fw.shadow[i] = core::ptr::read_volatile(src.add(i));
        }
    }
    fw.dirty = true;
    true
}

pub fn poll_event(id: u64) -> Option<WinEvent> {
    let mut guard = FOREIGN.lock();
    let fw = guard.iter_mut().find(|f| f.id == id)?;
    fw.events.pop_front()
}

pub fn destroy(id: u64) -> bool {
    let mut guard = FOREIGN.lock();
    if let Some(pos) = guard.iter().position(|f| f.id == id) {
        guard.remove(pos);
        true
    } else {
        false
    }
}

pub fn destroy_for_pid(pid: u64) {
    let mut guard = FOREIGN.lock();
    guard.retain(|f| f.pid != pid);
}

pub fn push_event_to_pid(pid: u64, ev: WinEvent) {
    let mut guard = FOREIGN.lock();
    for fw in guard.iter_mut() {
        if fw.pid == pid {
            fw.events.push_back(ev);
            return;
        }
    }
}

/// Попадание мыши в foreign-окно.
/// Возвращает `(id, buffer_x, buffer_y)`, где `buffer_x/y` — координаты
/// в буфере user-программы (без учёта заголовка).
pub fn hit_test(mx: i32, my: i32) -> Option<(u64, i32, i32)> {
    let guard = FOREIGN.lock();
    for fw in guard.iter().rev() {
        if !fw.alive { continue; }
        let x0 = fw.x;
        let y0 = fw.y;
        let x1 = x0 + fw.w as i32;
        let y1 = y0 + fw.h as i32 + TITLE_BAR_H;
        if mx >= x0 && my >= y0 && mx < x1 && my < y1 {
            // buffer_y = my - y0 - TITLE_BAR_H. Клик по заголовку даст
            // отрицательный buffer_y — user-программа сама решит, что с ним делать.
            return Some((fw.id, mx - x0, my - y0 - TITLE_BAR_H));
        }
    }
    None
}

pub fn with_windows<R>(f: impl FnOnce(&Vec<ForeignWindow>) -> R) -> R {
    let guard = FOREIGN.lock();
    f(&guard)
}

pub fn pid_of(id: u64) -> Option<u64> {
    let guard = FOREIGN.lock();
    guard.iter().find(|f| f.id == id).map(|f| f.pid)
}