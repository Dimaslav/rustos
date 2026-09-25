//! Блокирующий мьютекс для kernel-потоков.
//!
//! НЕЛЬЗЯ использовать из IRQ-обработчика: `lock()` усыпляет поток,
//! а в IRQ спать нельзя. Для IRQ — `spin::Mutex`.
//!
//! Реализация: атомарный флаг + FIFO-очередь waiters. Владелец на `unlock`
//! будит одного — тот повторяет `compare_exchange`, и либо захватывает, либо
//! снова уходит в очередь (это нормально, не строгая FIFO, но прогресс есть).

use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

use spin::Mutex as SpinMutex;

use super::thread::ThreadId;

pub struct ThreadMutex<T: ?Sized> {
    locked: AtomicBool,
    waiters: SpinMutex<Vec<ThreadId>>,
    data: UnsafeCell<T>,
}

unsafe impl<T: ?Sized + Send> Send for ThreadMutex<T> {}
unsafe impl<T: ?Sized + Send> Sync for ThreadMutex<T> {}

impl<T> ThreadMutex<T> {
    pub const fn new(data: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            waiters: SpinMutex::new(Vec::new()),
            data: UnsafeCell::new(data),
        }
    }
}

impl<T: ?Sized> ThreadMutex<T> {
    pub fn lock(&self) -> ThreadMutexGuard<'_, T> {
        loop {
            if self
                .locked
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return ThreadMutexGuard { mutex: self };
            }
            let me = crate::sched::current_id();
            self.waiters.lock().push(me);
            crate::sched::sleep_forever();
        }
    }

    pub fn try_lock(&self) -> Option<ThreadMutexGuard<'_, T>> {
        if self
            .locked
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            Some(ThreadMutexGuard { mutex: self })
        } else {
            None
        }
    }

    fn unlock(&self) {
        self.locked.store(false, Ordering::Release);
        if let Some(id) = self.waiters.lock().pop() {
            crate::sched::wake_thread(id);
        }
    }
}

pub struct ThreadMutexGuard<'a, T: ?Sized> {
    mutex: &'a ThreadMutex<T>,
}

impl<T: ?Sized> Deref for ThreadMutexGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.mutex.data.get() }
    }
}

impl<T: ?Sized> DerefMut for ThreadMutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.mutex.data.get() }
    }
}

impl<T: ?Sized> Drop for ThreadMutexGuard<'_, T> {
    fn drop(&mut self) {
        self.mutex.unlock();
    }
}