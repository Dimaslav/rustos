//! Round-robin scheduler с поддержкой per-process address space.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use spin::Mutex;
use x86_64::{
    registers::control::{Cr3, Cr3Flags},
    structures::paging::{PhysFrame, Size4KiB},
    PhysAddr,
};

use super::context::switch_context;
use super::thread::{Thread, ThreadId, ThreadState};

static KERNEL_CR3: AtomicU64 = AtomicU64::new(0);
static CURRENT_CR3: AtomicU64 = AtomicU64::new(0);

struct Scheduler {
    threads: Vec<Box<Thread>>,
    current: usize,
}

impl Scheduler {
    const fn new() -> Self { Self { threads: Vec::new(), current: 0 } }

    fn wake_expired(&mut self, now: u64) {
        for t in self.threads.iter_mut() {
            if t.state == ThreadState::Sleeping && t.wake_at != u64::MAX && t.wake_at <= now {
                t.state = ThreadState::Ready;
                t.wake_at = 0;
            }
        }
    }

    fn pick_next(&self) -> Option<usize> {
        let n = self.threads.len();
        if n == 0 { return None; }
        let mut idx = (self.current + 1) % n;
        for _ in 0..n {
            let t = &self.threads[idx];
            if !t.is_idle && (t.state == ThreadState::Ready || t.state == ThreadState::Running) {
                return Some(idx);
            }
            idx = (idx + 1) % n;
        }
        for _ in 0..n {
            let t = &self.threads[idx];
            if t.is_idle && (t.state == ThreadState::Ready || t.state == ThreadState::Running) {
                return Some(idx);
            }
            idx = (idx + 1) % n;
        }
        None
    }
}

static SCHED: Mutex<Option<Scheduler>> = Mutex::new(None);

fn perform_switch(
    old_rsp_ptr: *mut u64,
    next_rsp: u64,
    next_stack_top: u64,
    next_pml4: Option<u64>,
) {
    if next_stack_top != 0 {
        crate::gdt::set_rsp0(next_stack_top);
    }
    let target = next_pml4.unwrap_or_else(|| KERNEL_CR3.load(Ordering::Acquire));
    let current = CURRENT_CR3.load(Ordering::Acquire);
    if target != current && target != 0 {
        unsafe {
            let frame = PhysFrame::<Size4KiB>::containing_address(PhysAddr::new(target));
            Cr3::write(frame, Cr3Flags::empty());
        }
        CURRENT_CR3.store(target, Ordering::Release);
    }
    unsafe {
        switch_context(old_rsp_ptr, next_rsp);
    }
}

pub fn init() {
    let mut g = SCHED.lock();
    if g.is_some() { return; }
    let (frame, _flags) = Cr3::read();
    let kcr3 = frame.start_address().as_u64();
    KERNEL_CR3.store(kcr3, Ordering::Release);
    CURRENT_CR3.store(kcr3, Ordering::Release);
    let mut s = Scheduler::new();
    s.threads.push(Thread::main_kernel());
    s.threads.push(Thread::idle_kernel());
    s.current = 0;
    *g = Some(s);
}

pub fn spawn(name: &'static str, entry: extern "C" fn() -> !) -> ThreadId {
    spawn_inner(name, entry, None)
}

pub fn spawn_with_as(name: &'static str, entry: extern "C" fn() -> !, pml4_phys: u64) -> ThreadId {
    spawn_inner(name, entry, Some(pml4_phys))
}

fn spawn_inner(name: &'static str, entry: extern "C" fn() -> !, pml4_phys: Option<u64>) -> ThreadId {
    let parent = current_id();
    let thread = Thread::new_kernel(name, entry, pml4_phys, parent);
    let id = thread.id;
    let mut g = SCHED.lock();
    match g.as_mut() {
        Some(s) => s.threads.push(thread),
        None => panic!("sched::spawn до sched::init"),
    }
    id
}

pub fn current_id() -> ThreadId {
    let g = SCHED.lock();
    match g.as_ref() { Some(s) => s.threads[s.current].id, None => 0 }
}

pub fn current_name() -> &'static str {
    let g = SCHED.lock();
    match g.as_ref() { Some(s) => s.threads[s.current].name, None => "?" }
}

pub fn parent_of(pid: ThreadId) -> Option<ThreadId> {
    let g = SCHED.lock();
    let s = g.as_ref()?;
    s.threads.iter().find(|t| t.id == pid).map(|t| t.parent_id)
}

pub fn exit_code_of(pid: ThreadId) -> Option<i32> {
    let g = SCHED.lock();
    let s = g.as_ref()?;
    s.threads
        .iter()
        .find(|t| t.id == pid && t.state == ThreadState::Finished)
        .map(|t| t.exit_code)
}

pub fn wait_for(pid: ThreadId) -> Option<i32> {
    let mut guard = 0u32;
    loop {
        guard += 1;
        if guard > 300_000 { return None; }
        {
            let g = SCHED.lock();
            let s = g.as_ref()?;
            let t = s.threads.iter().find(|t| t.id == pid);
            match t {
                Some(t) if t.state == ThreadState::Finished => {
                    return Some(t.exit_code);
                }
                None => return None,
                _ => {}
            }
        }
        sleep_ms(10);
    }
}

pub fn kill(pid: ThreadId) -> bool {
    let mut g = SCHED.lock();
    let s = match g.as_mut() { Some(s) => s, None => return false };
    for t in s.threads.iter_mut() {
        if t.id == pid && t.state != ThreadState::Finished {
            t.state = ThreadState::Finished;
            t.exit_code = -9;
            return true;
        }
    }
    false
}

pub fn schedule_tick() {
    let now = crate::interrupts::ticks();
    let switch = {
        let mut g = SCHED.lock();
        let s = match g.as_mut() { Some(s) => s, None => return };
        s.wake_expired(now);
        let cur = s.current;
        let next = match s.pick_next() { Some(n) => n, None => return };
        if cur == next { return; }
        s.threads[cur].state = match s.threads[cur].state { ThreadState::Running => ThreadState::Ready, other => other };
        s.threads[next].state = ThreadState::Running;
        let ptr = s.threads[cur].rsp_slot();
        let nr = s.threads[next].rsp;
        let top = s.threads[next].kernel_stack_top();
        let pml4 = s.threads[next].pml4_phys;
        s.current = next;
        (ptr, nr, top, pml4)
    };
    perform_switch(switch.0, switch.1, switch.2, switch.3);
}

pub fn yield_now() {
    x86_64::instructions::interrupts::without_interrupts(|| {
        let switch = {
            let mut g = SCHED.lock();
            let s = match g.as_mut() { Some(s) => s, None => return };
            let cur = s.current;
            let next = match s.pick_next() { Some(n) => n, None => return };
            if cur == next { return; }
            s.threads[cur].state = match s.threads[cur].state { ThreadState::Running => ThreadState::Ready, other => other };
            s.threads[next].state = ThreadState::Running;
            let ptr = s.threads[cur].rsp_slot();
            let nr = s.threads[next].rsp;
            let top = s.threads[next].kernel_stack_top();
            let pml4 = s.threads[next].pml4_phys;
            s.current = next;
            (ptr, nr, top, pml4)
        };
        perform_switch(switch.0, switch.1, switch.2, switch.3);
    });
}

pub fn sleep_ms(ms: u64) {
    if ms == 0 { yield_now(); return; }
    x86_64::instructions::interrupts::without_interrupts(|| {
        let switch = {
            let mut g = SCHED.lock();
            let s = match g.as_mut() { Some(s) => s, None => return };
            let cur = s.current;
            let dticks = (ms * 18 / 1000).max(1);
            let wake_at = crate::interrupts::ticks().wrapping_add(dticks);
            s.threads[cur].state = ThreadState::Sleeping;
            s.threads[cur].wake_at = wake_at;
            let next = match s.pick_next() {
                Some(n) => n,
                None => { s.threads[cur].state = ThreadState::Running; s.threads[cur].wake_at = 0; return; }
            };
            if next == cur { s.threads[cur].state = ThreadState::Running; s.threads[cur].wake_at = 0; return; }
            s.threads[next].state = ThreadState::Running;
            let ptr = s.threads[cur].rsp_slot();
            let nr = s.threads[next].rsp;
            let top = s.threads[next].kernel_stack_top();
            let pml4 = s.threads[next].pml4_phys;
            s.current = next;
            (ptr, nr, top, pml4)
        };
        perform_switch(switch.0, switch.1, switch.2, switch.3);
    });
}

pub fn sleep_forever() {
    x86_64::instructions::interrupts::without_interrupts(|| {
        let switch = {
            let mut g = SCHED.lock();
            let s = match g.as_mut() { Some(s) => s, None => return };
            let cur = s.current;
            s.threads[cur].state = ThreadState::Sleeping;
            s.threads[cur].wake_at = u64::MAX;
            let next = match s.pick_next() {
                Some(n) => n,
                None => { s.threads[cur].state = ThreadState::Ready; s.threads[cur].wake_at = 0; return; }
            };
            if next == cur { s.threads[cur].state = ThreadState::Ready; s.threads[cur].wake_at = 0; return; }
            s.threads[next].state = ThreadState::Running;
            let ptr = s.threads[cur].rsp_slot();
            let nr = s.threads[next].rsp;
            let top = s.threads[next].kernel_stack_top();
            let pml4 = s.threads[next].pml4_phys;
            s.current = next;
            (ptr, nr, top, pml4)
        };
        perform_switch(switch.0, switch.1, switch.2, switch.3);
    });
}

pub fn wake_thread(id: ThreadId) {
    let mut g = SCHED.lock();
    if let Some(s) = g.as_mut() {
        for t in s.threads.iter_mut() {
            if t.id == id && t.state == ThreadState::Sleeping && t.wake_at == u64::MAX {
                t.state = ThreadState::Ready;
                t.wake_at = 0;
                return;
            }
        }
    }
}

pub fn exit_current_with_code(code: i32) -> ! {
    x86_64::instructions::interrupts::without_interrupts(|| {
        let switch = {
            let mut g = SCHED.lock();
            let s = match g.as_mut() { Some(s) => s, None => return };
            let cur = s.current;
            s.threads[cur].state = ThreadState::Finished;
            s.threads[cur].exit_code = code;
            let next = match s.pick_next() { Some(n) => n, None => return };
            s.threads[next].state = ThreadState::Running;
            let ptr = s.threads[cur].rsp_slot();
            let nr = s.threads[next].rsp;
            let top = s.threads[next].kernel_stack_top();
            let pml4 = s.threads[next].pml4_phys;
            s.current = next;
            (ptr, nr, top, pml4)
        };
        perform_switch(switch.0, switch.1, switch.2, switch.3);
    });
    loop { x86_64::instructions::hlt(); }
}

pub fn exit_current() -> ! {
    exit_current_with_code(0)
}

pub fn reap_finished() -> usize {
    x86_64::instructions::interrupts::without_interrupts(|| {
        // Сначала соберём id завершённых потоков, потом освободим их fd.
        let mut to_close_fds: Vec<ThreadId> = Vec::new();
        let mut g = SCHED.lock();
        let s = match g.as_mut() { Some(s) => s, None => return 0 };
        let cur_id = s.threads[s.current].id;

        let mut reaped = 0usize;
        let mut i = 0;
        while i < s.threads.len() {
            if s.threads[i].id != cur_id && s.threads[i].state == ThreadState::Finished {
                let has_waiter = s.threads.iter().any(|t| t.parent_id == s.threads[i].id);
                if has_waiter {
                    i += 1;
                    continue;
                }
                to_close_fds.push(s.threads[i].id);
                s.threads.remove(i);
                reaped += 1;
            } else {
                i += 1;
            }
        }
        if let Some(pos) = s.threads.iter().position(|t| t.id == cur_id) {
            s.current = pos;
        } else {
            s.current = 0;
        }
        drop(g);

        for pid in to_close_fds {
            crate::fd::close_all_for_pid(pid);
        }
        reaped
    })
}

// ---------- ps ----------

#[derive(Clone, Copy)]
pub struct ThreadInfo {
    pub id: ThreadId,
    pub state: u8,
    pub name: [u8; 32],
}

pub fn list_threads_snapshot() -> Vec<ThreadInfo> {
    let g = SCHED.lock();
    let s = match g.as_ref() {
        Some(s) => s,
        None => return Vec::new(),
    };
    let mut out = Vec::with_capacity(s.threads.len());
    for t in &s.threads {
        let state = match t.state {
            ThreadState::Ready => 0,
            ThreadState::Running => 1,
            ThreadState::Sleeping => 2,
            ThreadState::Finished => 3,
        };
        let mut name = [0u8; 32];
        let nb = t.name.as_bytes();
        let n = nb.len().min(31);
        name[..n].copy_from_slice(&nb[..n]);
        out.push(ThreadInfo { id: t.id, state, name });
    }
    out
}