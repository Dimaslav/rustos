//! Задача (Future) и её waker.

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::task::Wake;
use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use core::task::{Context, Poll, Waker};
use spin::Mutex;

pub type TaskId = u64;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

pub struct Task {
    id: TaskId,
    future: Mutex<Pin<Box<dyn Future<Output = ()> + Send + 'static>>>,
    queued: AtomicBool,
}

impl Task {
    pub fn new(future: impl Future<Output = ()> + Send + 'static) -> Arc<Self> {
        Arc::new(Task {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            future: Mutex::new(Box::pin(future)),
            queued: AtomicBool::new(false),
        })
    }

    pub fn id(&self) -> TaskId {
        self.id
    }

    /// Вызывается executor'ом. Возвращает true, если задача снова себя запросила.
    pub fn poll(&self) -> bool {
        let waker = Waker::from(Arc::new(TaskWaker {
            task_id: self.id,
        }));
        let mut cx = Context::from_waker(&waker);

        let mut fut = self.future.lock();
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(()) => false,
            Poll::Pending => true,
        }
    }

    /// Атомарно "забирает" флаг queued. true — если задачи ещё не было в очереди.
    pub fn take_queued(&self) -> bool {
        !self.queued.swap(true, Ordering::AcqRel)
    }

    /// Сбрасывает флаг queued (когда задача уже добавлена в очередь).
    pub fn clear_queued(&self) {
        self.queued.store(false, Ordering::Release);
    }
}

struct TaskWaker {
    task_id: TaskId,
}

impl Wake for TaskWaker {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        crate::task::executor::wake_task(self.task_id);
    }
}