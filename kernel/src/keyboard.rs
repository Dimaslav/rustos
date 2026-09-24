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
        if self.len == RING { return; } // переполнение — теряем событие
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

static MODS: Mutex<(bool, bool, bool)> = Mutex::new((false, false, false));
static EXTENDED: Mutex<bool> = Mutex::new(false);
static KEYS: Mutex<RingBuffer> = Mutex::new(RingBuffer::new());

pub fn modifiers() -> (bool, bool, bool) { *MODS.lock() }
pub fn pop() -> Option<Key> { KEYS.lock().pop() }

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

pub fn handle(sc: u8) {
    {
        let mut ext = EXTENDED.lock();
        if sc == 0xE0 { *ext = true; return; }
        if sc == 0xE1 { *ext = false; return; }
    }
    let is_ext = { let mut e = EXTENDED.lock(); let v = *e; *e = false; v };

    let release = sc & 0x80 != 0;
    let code = sc & 0x7F;

    match code {
        0x2A | 0x36 => { MODS.lock().0 = !release; return; }
        0x1D => { MODS.lock().1 = !release; return; }
        0x38 => { MODS.lock().2 = !release; return; }
        _ => {}
    }

    if release { return; }

    let key = match code {
        0x01 => Some(Key::Escape),
        0x0E => Some(Key::Backspace),
        0x0F => Some(Key::Tab),
        0x1C => Some(Key::Enter),
        0x48 if is_ext => Some(Key::Up),
        0x50 if is_ext => Some(Key::Down),
        0x4B if is_ext => Some(Key::Left),
        0x4D if is_ext => Some(Key::Right),
        0x47 if is_ext => Some(Key::Home),
        0x4F if is_ext => Some(Key::End),
        0x53 if is_ext => Some(Key::Delete),
        0x49 if is_ext => Some(Key::PageUp),
        0x51 if is_ext => Some(Key::PageDown),
        0x3B..=0x44 => Some(Key::F((code - 0x3B + 1) as u8)),
        0x57 => Some(Key::F(11)),
        0x58 => Some(Key::F(12)),
        _ => None,
    };
    if let Some(k) = key {
        KEYS.lock().push(k);
        return;
    }

    let shift = MODS.lock().0;
    if let Some(c) = scancode_to_ascii(code, shift) {
        KEYS.lock().push(Key::Char(c));
    }
}