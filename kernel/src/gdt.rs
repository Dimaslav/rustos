//! Global Descriptor Table + Task State Segment.

use alloc::boxed::Box;
use core::sync::atomic::{AtomicU64, Ordering};

use x86_64::instructions::tables::load_tss;
use x86_64::registers::segmentation::{Segment, CS, SS};
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;
use x86_64::VirtAddr;

pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;

const STACK_SIZE: usize = 4096 * 5; // 20 KiB

/// Выровнено на 16 — требование x86 для IST-указателя.
#[repr(align(16))]
struct Stack {
    inner: [u8; STACK_SIZE],
}

impl Stack {
    fn new() -> Box<Self> {
        Box::new(Stack { inner: [0u8; STACK_SIZE] })
    }
    fn top(&self) -> VirtAddr {
        VirtAddr::from_ptr(self.inner.as_ptr()) + STACK_SIZE as u64
    }
}

#[derive(Clone, Copy)]
struct Selectors {
    kernel_code: SegmentSelector,
    kernel_data: SegmentSelector,
    user_code: SegmentSelector,
    user_data: SegmentSelector,
    #[allow(dead_code)]
    tss: SegmentSelector,
}

static TSS_PTR: AtomicU64 = AtomicU64::new(0);
static SELECTORS: spin::Once<Selectors> = spin::Once::new();
static GDT_PTR: AtomicU64 = AtomicU64::new(0);
static DF_STACK_PTR: AtomicU64 = AtomicU64::new(0);

/// # Safety
/// До первого входа в Ring 3.
pub unsafe fn init() {
    let df_stack: &'static mut Stack = Box::leak(Stack::new());
    let df_stack_top = df_stack.top();
    DF_STACK_PTR.store(df_stack as *mut _ as u64, Ordering::Release);

    let tss: &'static mut TaskStateSegment = Box::leak(Box::new(TaskStateSegment::new()));
    tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = df_stack_top;
    tss.privilege_stack_table[0] = VirtAddr::new(0);
    TSS_PTR.store(tss as *mut _ as u64, Ordering::Release);

    let gdt: &'static mut GlobalDescriptorTable =
        Box::leak(Box::new(GlobalDescriptorTable::new()));
    let kernel_code = gdt.append(Descriptor::kernel_code_segment());
    let kernel_data = gdt.append(Descriptor::kernel_data_segment());
    let user_code = gdt.append(Descriptor::user_code_segment());
    let user_data = gdt.append(Descriptor::user_data_segment());
    let tss_sel = gdt.append(Descriptor::tss_segment(tss));
    GDT_PTR.store(gdt as *mut _ as u64, Ordering::Release);

    SELECTORS.call_once(|| Selectors {
        kernel_code,
        kernel_data,
        user_code,
        user_data,
        tss: tss_sel,
    });

    gdt.load();
    CS::set_reg(kernel_code);
    SS::set_reg(kernel_data);
    load_tss(tss_sel);
}

pub fn kernel_code_selector() -> SegmentSelector {
    SELECTORS.get().expect("GDT").kernel_code
}
pub fn kernel_data_selector() -> SegmentSelector {
    SELECTORS.get().expect("GDT").kernel_data
}
pub fn user_code_selector() -> SegmentSelector {
    SELECTORS.get().expect("GDT").user_code
}
pub fn user_data_selector() -> SegmentSelector {
    SELECTORS.get().expect("GDT").user_data
}

/// Установить kernel-стек для перехода Ring 3 → Ring 0.
pub fn set_rsp0(addr: u64) {
    let ptr = TSS_PTR.load(Ordering::Acquire);
    if ptr == 0 {
        return;
    }
    // Safety: TSS живёт вечно (Box::leak), пишем один u64.
    let tss = unsafe { &mut *(ptr as *mut TaskStateSegment) };
    tss.privilege_stack_table[0] = VirtAddr::new(addr);
}

pub fn code_selector() -> SegmentSelector {
    kernel_code_selector()
}
pub fn data_selector() -> SegmentSelector {
    kernel_data_selector()
}