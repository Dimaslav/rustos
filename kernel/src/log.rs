use core::fmt;
use core::sync::atomic::{AtomicU8, AtomicU64, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Level {
    Error = 1,
    Warn = 2,
    Info = 3,
    Debug = 4,
    Trace = 5,
}

impl Level {
    pub fn tag(self) -> &'static str {
        match self {
            Level::Error => "ERROR",
            Level::Warn  => "WARN ",
            Level::Info  => "INFO ",
            Level::Debug => "DEBUG",
            Level::Trace => "TRACE",
        }
    }
}

static LEVEL: AtomicU8 = AtomicU8::new(Level::Info as u8);

pub fn set_level(l: Level) {
    LEVEL.store(l as u8, Ordering::Relaxed);
}

pub fn level() -> Level {
    match LEVEL.load(Ordering::Relaxed) {
        1 => Level::Error,
        2 => Level::Warn,
        3 => Level::Info,
        4 => Level::Debug,
        _ => Level::Trace,
    }
}

#[inline]
pub fn enabled(l: Level) -> bool {
    (l as u8) <= LEVEL.load(Ordering::Relaxed)
}

// ---------- Ring buffer ----------

pub const LOG_LINE_MAX: usize = 200;
pub const LOG_RING_SIZE: usize = 64;

#[derive(Clone, Copy)]
struct LogEntry {
    tick: u64,
    len: u16,
    buf: [u8; LOG_LINE_MAX],
}

const EMPTY: LogEntry = LogEntry { tick: 0, len: 0, buf: [0; LOG_LINE_MAX] };

struct Ring {
    entries: [LogEntry; LOG_RING_SIZE],
    head: usize,
    count: usize,
}

static mut RING: Ring = Ring {
    entries: [EMPTY; LOG_RING_SIZE],
    head: 0,
    count: 0,
};

/// Спин-лок для ring buffer. Не берём spin::Mutex, потому что RING в
/// `static mut` — можно взять атомарный флаг.
static RING_LOCK: AtomicU8 = AtomicU8::new(0);
static DROPPED: AtomicU64 = AtomicU64::new(0);
static RING_INIT: AtomicU8 = AtomicU8::new(0);

fn ring_try_lock() -> bool {
    RING_LOCK.compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed).is_ok()
}

fn ring_unlock() {
    RING_LOCK.store(0, Ordering::Release);
}

unsafe fn ring_push(tick: u64, bytes: &[u8]) {
    let n = bytes.len().min(LOG_LINE_MAX);
    let head = RING.head;
    let e = &mut RING.entries[head];
    e.tick = tick;
    e.len = n as u16;
    e.buf[..n].copy_from_slice(&bytes[..n]);
    // Прочие байты не трогаем — len ограничивает.

    RING.head = (head + 1) % LOG_RING_SIZE;
    if RING.count < LOG_RING_SIZE {
        RING.count += 1;
    }
}

/// Снимок всего ring buffer'а в текстовом виде, от старых к новым.
pub fn snapshot() -> alloc::string::String {
    use alloc::string::String;
    let mut out = String::new();

    if !ring_try_lock() {
        out.push_str("(ring buffer busy)\n");
        return out;
    }
    unsafe {
        let dropped = DROPPED.load(Ordering::Relaxed);
        if dropped > 0 {
            out.push_str(&alloc::format!(
                "[kmsg] {} messages dropped\n", dropped
            ));
        }
        let count = RING.count;
        // Индекс самого старого: если count < size, это 0;
        // иначе head (там сейчас лежит самая старая запись).
        let start = if count < LOG_RING_SIZE {
            0
        } else {
            RING.head
        };
        for i in 0..count {
            let idx = (start + i) % LOG_RING_SIZE;
            let e = &RING.entries[idx];
            let n = e.len as usize;
            let s = core::str::from_utf8(&e.buf[..n]).unwrap_or("?");
            out.push_str(&alloc::format!("[{}] {}\n", e.tick, s));
        }
    }
    ring_unlock();
    out
}

// ---------- Emit ----------

#[doc(hidden)]
pub fn emit(l: Level, args: fmt::Arguments) {
    if !enabled(l) {
        return;
    }

    // 1. В serial — как раньше.
    crate::serial::_print(format_args!("[{}] {}\n", l.tag(), args));

    // 2. В ring buffer — без heap, простой формат.
    //    Format не поддерживает «сначала format, потом copy» в статику.
    //    Используем маленький стековый ArrayWriter.
    struct ArrayWriter {
        buf: [u8; LOG_LINE_MAX],
        len: usize,
    }
    impl fmt::Write for ArrayWriter {
        fn write_str(&mut self, s: &str) -> fmt::Result {
            let bytes = s.as_bytes();
            let take = (LOG_LINE_MAX - self.len).min(bytes.len());
            self.buf[self.len..self.len + take].copy_from_slice(&bytes[..take]);
            self.len += take;
            Ok(())
        }
    }

    let mut w = ArrayWriter { buf: [0u8; LOG_LINE_MAX], len: 0 };
    let _ = fmt::Write::write_fmt(&mut w, format_args!("{} {}", l.tag(), args));

    // Усечём до LOG_LINE_MAX-1 и добавим '\n'.
    let mut len = w.len;
    if len >= LOG_LINE_MAX {
        len = LOG_LINE_MAX - 1;
    }
    w.buf[len] = b'\n';
    len += 1;

    if ring_try_lock() {
        let tick = crate::interrupts::ticks();
        unsafe { ring_push(tick, &w.buf[..len]) };
        ring_unlock();
    } else {
        DROPPED.fetch_add(1, Ordering::Relaxed);
    }

    // Подавляем warning про RING_INIT (используется только для отладки).
    let _ = RING_INIT.load(Ordering::Relaxed);
}

// ---------- Макросы ----------

#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => {
        $crate::log::emit($crate::log::Level::Error, format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => {
        $crate::log::emit($crate::log::Level::Warn, format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {
        $crate::log::emit($crate::log::Level::Info, format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_debug {
    ($($arg:tt)*) => {
        $crate::log::emit($crate::log::Level::Debug, format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_trace {
    ($($arg:tt)*) => {
        $crate::log::emit($crate::log::Level::Trace, format_args!($($arg)*))
    };
}