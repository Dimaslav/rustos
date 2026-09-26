//! Поток и его kernel-стек.

use alloc::alloc::{alloc_zeroed, dealloc, Layout};
use alloc::boxed::Box;
use core::sync::atomic::{AtomicU64, Ordering};

pub type ThreadId = u64;

pub const STACK_SIZE: usize = 16 * 1024;
const STACK_ALIGN: usize = 16;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ThreadState {
    Ready,
    Running,
    Sleeping,
    Finished,
}

pub struct Thread {
    pub id: ThreadId,
    pub parent_id: ThreadId,
    pub exit_code: i32,
    pub name: &'static str,
    pub rsp: u64,
    stack_ptr: usize,
    pub state: ThreadState,
    pub wake_at: u64,
    pub is_idle: bool,
    pub(in crate::sched) pml4_phys: Option<u64>,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

impl Thread {
    pub fn new_kernel(
        name: &'static str,
        entry: extern "C" fn() -> !,
        pml4_phys: Option<u64>,
        parent_id: ThreadId,
    ) -> Box<Thread> {
        let layout = Layout::from_size_align(STACK_SIZE, STACK_ALIGN).unwrap();
        let stack_ptr = unsafe { alloc_zeroed(layout) };
        if stack_ptr.is_null() {
            panic!("thread stack allocation failed ({} bytes)", STACK_SIZE);
        }
        let stack_top = stack_ptr as u64 + STACK_SIZE as u64;
        let rsp = unsafe { init_stack(stack_top, entry) };

        Box::new(Thread {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            parent_id,
            exit_code: 0,
            name,
            rsp,
            stack_ptr: stack_ptr as usize,
            state: ThreadState::Ready,
            wake_at: 0,
            is_idle: false,
            pml4_phys,
        })
    }

    pub fn main_kernel() -> Box<Thread> {
        Box::new(Thread {
            id: 0,
            parent_id: 0,
            exit_code: 0,
            name: "main",
            rsp: 0,
            stack_ptr: 0,
            state: ThreadState::Running,
            wake_at: 0,
            is_idle: false,
            pml4_phys: None,
        })
    }

    pub fn idle_kernel() -> Box<Thread> {
        let layout = Layout::from_size_align(STACK_SIZE, STACK_ALIGN).unwrap();
        let stack_ptr = unsafe { alloc_zeroed(layout) };
        if stack_ptr.is_null() {
            panic!("idle stack allocation failed");
        }
        let stack_top = stack_ptr as u64 + STACK_SIZE as u64;
        let rsp = unsafe { init_stack(stack_top, idle_entry) };

        Box::new(Thread {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            parent_id: 0,
            exit_code: 0,
            name: "idle",
            rsp,
            stack_ptr: stack_ptr as usize,
            state: ThreadState::Ready,
            wake_at: 0,
            is_idle: true,
            pml4_phys: None,
        })
    }

    pub fn rsp_slot(&mut self) -> *mut u64 {
        &mut self.rsp
    }

    pub fn kernel_stack_top(&self) -> u64 {
        if self.stack_ptr == 0 {
            0
        } else {
            self.stack_ptr as u64 + STACK_SIZE as u64
        }
    }
}

impl Drop for Thread {
    fn drop(&mut self) {
        crate::win::destroy_for_pid(self.id);

        if self.stack_ptr != 0 {
            let layout = Layout::from_size_align(STACK_SIZE, STACK_ALIGN).unwrap();
            unsafe { dealloc(self.stack_ptr as *mut u8, layout) };
            self.stack_ptr = 0;
        }
        if let Some(pml4) = self.pml4_phys {
            unsafe {
                crate::memory::AddressSpace::destroy(pml4);
            }
            self.pml4_phys = None;
        }
    }
}

extern "C" fn idle_entry() -> ! {
    x86_64::instructions::interrupts::enable();
    loop {
        x86_64::instructions::hlt();
    }
}

unsafe fn init_stack(stack_top: u64, entry: extern "C" fn() -> !) -> u64 {
    debug_assert!(stack_top % 16 == 0);
    let base = (stack_top - 64) as *mut u64;
    for i in 0..6 {
        *base.add(i) = 0;
    }
    *base.add(6) = entry as u64;
    *base.add(7) = 0;
    base as u64
}