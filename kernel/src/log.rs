//! Логирование с уровнями.
//!
//! Уровни (от важного к шумному): `Error < Warn < Info < Debug < Trace`.
//! Глобальный порог задаётся через `set_level`. Всё, что ниже порога,
//! отбрасывается без форматирования.
//!
//! Использование:
//! ```ignore
//! use crate::{log_info, log_warn, log_error, log_debug};
//! log_info!("[boot] serial online");
//! log_warn!("[pic] IRQ2 masked");
//! log_error!("[elf] load failed: {:?}", err);
//! ```
//!
//! Если serial ещё не инициализирован, `serial_print!` просто ничего не делает
//! (см. `Serial::_print` — он пропускает запись, если `SERIAL.lock()` вернёт `None`).

use core::fmt;
use core::sync::atomic::{AtomicU8, Ordering};

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

/// Порог по умолчанию. Можно поменять из `kernel_main`.
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

/// Основная точка входа. Не вызывай напрямую — используй макросы ниже.
#[doc(hidden)]
pub fn emit(l: Level, args: fmt::Arguments) {
    if !enabled(l) {
        return;
    }
    // Пишем напрямую через serial, без lock-а внутри lock-а.
    crate::serial::_print(format_args!("[{}] {}\n", l.tag(), args));
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