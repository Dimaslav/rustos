//! Future, завершающийся по достижении указанного тика.

use alloc::vec::Vec;
use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::{AtomicU64, Ordering};
use core::task::{Context, Poll, Waker};
use spin::Mutex;

use crate::interrupts::ticks;

struct Sleeper {
    deadline: u64,
    waker: Option<Waker>,
}

static SLEEPERS: Mutex<Vec<Sleeper>> = Mutex::new(Vec::new());
static GENERATION: AtomicU64 = AtomicU64::new(0);

pub struct Sleep {
    deadline: u64,
    registered: bool,
}

impl Sleep {
    pub fn new_ms(ms: u64) -> Self {
        // PIT ~18.2 Гц → 1 сек ≈ 18 тиков. ms → ticks: ms * 18 / 1000.
        let dticks = (ms * 18 / 1000).max(1);
        Self {
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

        if !self.registered {
            let mut s = SLEEPERS.lock();
            s.push(Sleeper {
                deadline: self.deadline,
                waker: Some(cx.waker().clone()),
            });
            self.registered = true;
            GENERATION.fetch_add(1, Ordering::Relaxed);
        } else {
            // Обновляем waker, если он изменился.
            let mut s = SLEEPERS.lock();
            for sl in s.iter_mut() {
                if sl.deadline == self.deadline {
                    sl.waker = Some(cx.waker().clone());
                    break;
                }
            }
        }

        Poll::Pending
    }
}

/// Вызывается из timer IRQ. Пробуждает все sleepers, чей deadline наступил.
pub fn wake_expired() {
    let now = ticks();
    let mut to_wake: Vec<Waker> = Vec::new();
    {
        let mut s = SLEEPERS.lock();
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
    }
    for w in to_wake {
        w.wake();
    }
    let _ = GENERATION.load(Ordering::Relaxed);
}