use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};
use spin::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Char(u8),
    Enter,
    Backspace,
    Tab,
    Escape,
    Up, Down, Left, Right,
    Home, End, Delete, PageUp, PageDown,
    F(u8),
}

const RING: usize = 256;

struct RingBuffer {
    buf: [Option<Key>; RING],
    head: usize,
    tail: usize,
    len: usize,
}

impl RingBuffer {
    const fn new() -> Self {
        Self { buf: [None; RING], head: 0, tail: 0, len: 0 }
    }
    fn push(&mut self, k: Key) {
        if self.len == RING { return; }
        self.buf[self.tail] = Some(k);
        self.tail = (self.tail + 1) % RING;
        self.len += 1;
    }
    fn pop(&mut self) -> Option<Key> {
        if self.len == 0 { return None; }
        let k = self.buf[self.head].take();
        self.head = (self.head + 1) % RING;
        self.len -= 1;
        k
    }
}

/// Одна зажатая клавиша. Нужна для автоповтора в `task::repeat`.
struct HeldKey {
    /// Сканкод без release-бита (0x00..0x7F).
    scancode: u8,
    /// Был ли префикс 0xE0.
    is_ext: bool,
    key: Key,
    start_tick: u64,
}

static MODS: Mutex<(bool, bool, bool)> = Mutex::new((false, false, false));
static EXTENDED: Mutex<bool> = Mutex::new(false);
static KEYS: Mutex<RingBuffer> = Mutex::new(RingBuffer::new());
/// Набор зажатых клавиш (а не одна) — иначе отпускание одной сбрасывало бы
/// автоповтор для другой.
static HELD: Mutex<Vec<HeldKey>> = Mutex::new(Vec::new());

/// Если `true`, GUI не забирает клавиатуру — ею владеет user-shell.
static USER_OWNS: AtomicBool = AtomicBool::new(false);

pub fn modifiers() -> (bool, bool, bool) { *MODS.lock() }
pub fn pop() -> Option<Key> { KEYS.lock().pop() }

/// Первая (самая старая) зажатая клавиша — именно она повторяется.
pub fn held() -> Option<(Key, u64)> {
    let h = HELD.lock();
    h.first().map(|e| (e.key, e.start_tick))
}

pub fn push_key(k: Key) { KEYS.lock().push(k); }

pub fn set_user_owns(v: bool) { USER_OWNS.store(v, Ordering::Release); }
pub fn user_owns() -> bool { USER_OWNS.load(Ordering::Acquire) }

/// Сбросить состояние зажатых клавиш (например, при потере фокуса).
pub fn clear_held() {
    HELD.lock().clear();
}

fn add_held(scancode: u8, is_ext: bool, key: Key) {
    let now = crate::interrupts::ticks();
    let mut h = HELD.lock();
    // Убираем старую запись для того же физического ключа (на всякий случай),
    // затем добавляем свежую.
    h.retain(|e| !(e.scancode == scancode && e.is_ext == is_ext));
    h.push(HeldKey { scancode, is_ext, key, start_tick: now });
}

fn remove_held(scancode: u8, is_ext: bool) {
    HELD.lock().retain(|e| !(e.scancode == scancode && e.is_ext == is_ext));
}

/// Достать «печатный» символ из очереди:
/// `Enter` → `'\n'`, `Backspace` → `0x08`, `Tab` → `'\t'`.
/// Всё остальное (стрелки, F-клавиши, Escape) пропускается.
pub fn pop_char() -> Option<u8> {
    loop {
        let k = pop()?;
        match k {
            Key::Char(c) => return Some(c),
            Key::Enter => return Some(b'\n'),
            Key::Backspace => return Some(0x08),
            Key::Tab => return Some(b'\t'),
            _ => continue,
        }
    }
}

pub fn scancode_to_ascii(sc: u8, shift: bool) -> Option<u8> {
    let c = match sc {
        0x02 => if shift { b'!' } else { b'1' },
        0x03 => if shift { b'@' } else { b'2' },
        0x04 => if shift { b'#' } else { b'3' },
        0x05 => if shift { b'$' } else { b'4' },
        0x06 => if shift { b'%' } else { b'5' },
        0x07 => if shift { b'^' } else { b'6' },
        0x08 => if shift { b'&' } else { b'7' },
        0x09 => if shift { b'*' } else { b'8' },
        0x0A => if shift { b'(' } else { b'9' },
        0x0B => if shift { b')' } else { b'0' },
        0x0C => if shift { b'_' } else { b'-' },
        0x0D => if shift { b'+' } else { b'=' },
        0x0E => 0x08,
        0x10 => if shift { b'Q' } else { b'q' },
        0x11 => if shift { b'W' } else { b'w' },
        0x12 => if shift { b'E' } else { b'e' },
        0x13 => if shift { b'R' } else { b'r' },
        0x14 => if shift { b'T' } else { b't' },
        0x15 => if shift { b'Y' } else { b'y' },
        0x16 => if shift { b'U' } else { b'u' },
        0x17 => if shift { b'I' } else { b'i' },
        0x18 => if shift { b'O' } else { b'o' },
        0x19 => if shift { b'P' } else { b'p' },
        0x1A => if shift { b'{' } else { b'[' },
        0x1B => if shift { b'}' } else { b']' },
        0x1C => b'\n',
        0x1E => if shift { b'A' } else { b'a' },
        0x1F => if shift { b'S' } else { b's' },
        0x20 => if shift { b'D' } else { b'd' },
        0x21 => if shift { b'F' } else { b'f' },
        0x22 => if shift { b'G' } else { b'g' },
        0x23 => if shift { b'H' } else { b'h' },
        0x24 => if shift { b'J' } else { b'j' },
        0x25 => if shift { b'K' } else { b'k' },
        0x26 => if shift { b'L' } else { b'l' },
        0x27 => if shift { b':' } else { b';' },
        0x28 => if shift { b'"' } else { b'\'' },
        0x29 => if shift { b'~' } else { b'`' },
        0x2B => if shift { b'|' } else { b'\\' },
        0x2C => if shift { b'Z' } else { b'z' },
        0x2D => if shift { b'X' } else { b'x' },
        0x2E => if shift { b'C' } else { b'c' },
        0x2F => if shift { b'V' } else { b'v' },
        0x30 => if shift { b'B' } else { b'b' },
        0x31 => if shift { b'N' } else { b'n' },
        0x32 => if shift { b'M' } else { b'm' },
        0x33 => if shift { b'<' } else { b',' },
        0x34 => if shift { b'>' } else { b'.' },
        0x35 => if shift { b'?' } else { b'/' },
        0x39 => b' ',
        _ => return None,
    };
    Some(c)
}

/// Специальные (непечатные) клавиши. `is_ext` важен: без префикса 0xE0
/// коды 0x48/0x50/... означают цифры на numpad, а не стрелки.
fn decode_special(code: u8, is_ext: bool) -> Option<Key> {
    Some(match code {
        0x01 => Key::Escape,
        0x0E => Key::Backspace,
        0x0F => Key::Tab,
        0x1C => Key::Enter,
        0x48 if is_ext => Key::Up,
        0x50 if is_ext => Key::Down,
        0x4B if is_ext => Key::Left,
        0x4D if is_ext => Key::Right,
        0x47 if is_ext => Key::Home,
        0x4F if is_ext => Key::End,
        0x53 if is_ext => Key::Delete,
        0x49 if is_ext => Key::PageUp,
        0x51 if is_ext => Key::PageDown,
        0x3B..=0x44 => Key::F(code - 0x3B + 1),
        0x57 => Key::F(11),
        0x58 => Key::F(12),
        _ => return None,
    })
}

pub fn handle(sc: u8) {
    // 1. Разбираемся с префиксом 0xE0.
    let is_ext = {
        let mut e = EXTENDED.lock();
        if sc == 0xE0 {
            *e = true;
            return;
        }
        if sc == 0xE1 {
            // Pause/Break — много-байтовая последовательность, пропускаем.
            *e = false;
            return;
        }
        let v = *e;
        *e = false;
        v
    };

    let release = sc & 0x80 != 0;
    let code = sc & 0x7F;

    // 2. Модификаторы — не участвуют в автоповторе.
    match code {
        0x2A | 0x36 => { MODS.lock().0 = !release; return; } // Shift
        0x1D => { MODS.lock().1 = !release; return; }        // Ctrl
        0x38 => { MODS.lock().2 = !release; return; }        // Alt
        _ => {}
    }

    // 3. Release — убираем конкретный физический ключ из набора зажатых.
    if release {
        remove_held(code, is_ext);
        return;
    }

    // 4. Специальная клавиша?
    if let Some(k) = decode_special(code, is_ext) {
        KEYS.lock().push(k);
        // Escape и F-клавиши не повторяются.
        if !matches!(k, Key::Escape | Key::F(_)) {
            add_held(code, is_ext, k);
        }
        return;
    }

    // 5. Обычный символ.
    let shift = MODS.lock().0;
    if let Some(c) = scancode_to_ascii(code, shift) {
        let k = Key::Char(c);
        KEYS.lock().push(k);
        add_held(code, is_ext, k);
    }
}