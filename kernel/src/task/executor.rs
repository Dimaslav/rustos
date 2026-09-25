//! Простой однопоточный executor.
//!
//! Модель: задача попадает в `ready` при spawn и при каждом `wake()`.
//! `run_ready()` опрашивает все готовые задачи, не удерживая lock на
//! таблице задач во время `poll()` — иначе получим дедлок, если задача
//! вызовет `spawn()` или разбудит саму себя.
//!
//! Голодание невозможно: каждая задача, вернувшая `Pending`, обязана
//! зарегистрировать waker (см. `Sleep`), который вернёт её в `ready`.

use alloc::collections::{BTreeMap, VecDeque};
use alloc::sync::Arc;
use alloc::vec::Vec;
use spin::Mutex;

use super::task::{Task, TaskId};

pub struct Executor {
    tasks: Mutex<BTreeMap<TaskId, Arc<Task>>>,
    ready: Mutex<VecDeque<TaskId>>,
}

impl Executor {
    pub const fn new() -> Self {
        Self {
            tasks: Mutex::new(BTreeMap::new()),
            ready: Mutex::new(VecDeque::new()),
        }
    }

    pub fn spawn(&self, task: Arc<Task>) {
        let id = task.id();
        task.clear_queued();
        self.tasks.lock().insert(id, task);
        self.ready.lock().push_back(id);
    }

    /// Вызывается из waker'а. Идемпотентно: если задача уже в очереди,
    /// повторный `wake()` не создаёт дубликат.
    pub fn enqueue(&self, id: TaskId) {
        let task = match self.tasks.lock().get(&id) {
            Some(t) => t.clone(),
            None => return,
        };
        // take_queued(): false → true + вернёт true (первый wake)
        if task.take_queued() {
            self.ready.lock().push_back(id);
        }
    }

    /// Опросить все готовые задачи. Возвращает число polled.
    ///
    /// Не держит lock на `tasks` во время `poll()` — задача может
    /// вызывать `spawn()`, `enqueue()` и т.д.
    pub fn run_ready(&self) -> usize {
        let to_run: Vec<TaskId> = {
            let mut ready = self.ready.lock();
            ready.drain(..).collect()
        };

        let mut count = 0;
        for id in to_run {
            // Клонируем Arc — освобождаем lock немедленно.
            let task = match self.tasks.lock().get(&id).cloned() {
                Some(t) => t,
                None => continue,
            };

            task.clear_queued();
            count += 1;

            if task.poll() {
                // Pending — задача остаётся в tasks, ждёт следующего wake().
                // Если future не зарегистрировал waker — это баг future,
                // а не executor'а.
            } else {
                // Ready — задача завершилась, удаляем.
                self.tasks.lock().remove(&id);
            }
        }
        count
    }
}

// ---------- Глобальный executor ----------

pub static EXECUTOR: Executor = Executor::new();

pub fn spawn(future: impl core::future::Future<Output = ()> + Send + 'static) {
    let task = Task::new(future);
    EXECUTOR.spawn(task);
}

/// Вызывается из waker'а.
pub fn wake_task(id: TaskId) {
    EXECUTOR.enqueue(id);
}

/// Вызывается из главного цикла.
pub fn run_ready() -> usize {
    EXECUTOR.run_ready()
}