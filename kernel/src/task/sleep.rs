//! Future, завершающийся по достижении указанного тика.
//!
//! # IRQ-safety
//!
//! `wake_expired()` вызывается из timer IRQ, а `poll()` — из главного цикла
//! с включёнными прерываниями. Чтобы они не ловили взаимный spin на
//! `SLEEPERS`, **все** обращения к нему идут под `without_interrupts`.
//! Из IRQ-контекста это no-op (флаг уже снят), так что вложенность безопасна.

use alloc::vec::Vec;
use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::{AtomicU64, Ordering};
use core::task::{Context, Poll, Waker};
use spin::Mutex;

use crate::interrupts::ticks;

struct Sleeper {
    id: u64,
    deadline: u64,
    waker: Option<Waker>,
}

static SLEEPERS: Mutex<Vec<Sleeper>> = Mutex::new(Vec::new());
static NEXT_SLEEPER_ID: AtomicU64 = AtomicU64::new(1);

/// Общий вход в SLEEPERS с запретом прерываний.
fn with_sleepers<R>(f: impl FnOnce(&mut Vec<Sleeper>) -> R) -> R {
    x86_64::instructions::interrupts::without_interrupts(|| {
        let mut g = SLEEPERS.lock();
        f(&mut g)
    })
}

pub struct Sleep {
    id: u64,
    deadline: u64,
    registered: bool,
}

impl Sleep {
    pub fn new_ms(ms: u64) -> Self {
        let dticks = (ms * 18 / 1000).max(1);
        Self {
            id: NEXT_SLEEPER_ID.fetch_add(1, Ordering::Relaxed),
            deadline: ticks() + dticks,
            registered: false,
        }
    }
}

impl Future for Sleep {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let now = ticks();
        if now >= self.deadline {
            return Poll::Ready(());
        }

        let id = self.id;
        let deadline = self.deadline;
        let registered = self.registered;
        let waker = cx.waker().clone();

        with_sleepers(|s| {
            if !registered {
                // ИСПРАВЛЕНИЕ: было `waker`, теперь `waker: Some(waker)`.
                s.push(Sleeper { id, deadline, waker: Some(waker) });
            } else {
                for sl in s.iter_mut() {
                    if sl.id == id {
                        sl.waker = Some(waker);
                        return;
                    }
                }
                // Не нашли — например, deadline уже наступил и wake_expired
                // удалил запись, но ticks() в начале poll показал < deadline
                // из-за гонки. Регистрируем заново.
                s.push(Sleeper { id, deadline, waker: None });
            }
        });

        if !self.registered {
            self.registered = true;
        }
        Poll::Pending
    }
}

/// Вызывается из timer IRQ. Пробуждает все sleepers, чей deadline наступил.
pub fn wake_expired() {
    let now = ticks();
    let mut to_wake: Vec<Waker> = Vec::new();
    with_sleepers(|s| {
        let mut i = 0;
        while i < s.len() {
            if s[i].deadline <= now {
                if let Some(w) = s[i].waker.take() {
                    to_wake.push(w);
                }
                s.swap_remove(i);
            } else {
                i += 1;
            }
        }
    });
    for w in to_wake {
        w.wake();
    }
}