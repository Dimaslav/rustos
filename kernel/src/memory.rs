use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;
use bootloader_api::info::{MemoryRegionKind, MemoryRegions};
use core::sync::atomic::{AtomicU64, Ordering};
use spin::Mutex;
use x86_64::{
    instructions::tlb,
    registers::control::Cr3,
    registers::model_specific::{Efer, EferFlags},
    structures::paging::{
        FrameAllocator, FrameDeallocator, OffsetPageTable, PageTable, PageTableFlags,
        PhysFrame, Size4KiB, Translate,
    },
    PhysAddr, VirtAddr,
};

const LOW_MEM_LIMIT: u64 = 0x10_0000;

pub const USER_PML4_INDEX: usize = 0;
pub const USER_ADDR_MAX: u64 = 0x0000_8000_0000_0000;

const PAGE_USER: u64 = 1 << 2;
const PDE_HUGE: u64 = 1 << 7;
const ADDR_MASK: u64 = 0x000F_FFFF_FFFF_F000;
const HUGE_ADDR_MASK: u64 = 0x000F_FFFF_FFE0_0000;
const PAGE_NX: u64 = 1 << 63;
const PAGE_WRITABLE: u64 = 1 << 1;
const PAGE_PRESENT: u64 = 1 << 0;

/// Software-бит в PTE: страница помечена copy-on-write.
/// Бит 9 доступен ОС (в x86_64 биты 9-11 не используются железом).
pub const PTE_COW_BIT: u64 = 1 << 9;

/// Refcounts на физические фреймы, разделяемые между AS (COW).
/// Запись появляется только когда фрейм становится разделяемым (>1 владельца).
static COW_REFS: Mutex<BTreeMap<u64, u32>> = Mutex::new(BTreeMap::new());

pub static FRAME_ALLOCATOR: Mutex<Option<BootInfoFrameAllocator>> = Mutex::new(None);
pub static KERNEL_PML4_PHYS: AtomicU64 = AtomicU64::new(0);
pub static PHYS_OFFSET: AtomicU64 = AtomicU64::new(0);

pub unsafe fn enable_nx() {
    let mut flags = Efer::read();
    if !flags.contains(EferFlags::NO_EXECUTE_ENABLE) {
        flags.insert(EferFlags::NO_EXECUTE_ENABLE);
        Efer::write(flags);
    }
}

pub unsafe fn init(physical_memory_offset: VirtAddr) -> OffsetPageTable<'static> {
    enable_nx();
    PHYS_OFFSET.store(physical_memory_offset.as_u64(), Ordering::Release);
    let (frame, _flags) = Cr3::read();
    KERNEL_PML4_PHYS.store(frame.start_address().as_u64(), Ordering::Release);
    let level_4_table = active_level_4_table(physical_memory_offset);
    OffsetPageTable::new(level_4_table, physical_memory_offset)
}

unsafe fn active_level_4_table(physical_memory_offset: VirtAddr) -> &'static mut PageTable {
    let (level_4_table_frame, _) = Cr3::read();
    let phys = level_4_table_frame.start_address();
    let virt = physical_memory_offset + phys.as_u64();
    let page_table_ptr: *mut PageTable = virt.as_mut_ptr();
    &mut *page_table_ptr
}

pub unsafe fn is_user_range_mapped(start: u64, len: usize) -> bool {
    if len == 0 { return true; }
    if start == 0 { return false; }
    if start >= USER_ADDR_MAX { return false; }
    match start.checked_add(len as u64) {
        Some(end) if end <= USER_ADDR_MAX => {}
        _ => return false,
    }
    let phys_offset_u64 = PHYS_OFFSET.load(Ordering::Acquire);
    if phys_offset_u64 == 0 { return false; }
    let phys_offset = VirtAddr::new(phys_offset_u64);
    let l4 = active_level_4_table(phys_offset);
    let mapper = OffsetPageTable::new(l4, phys_offset);
    let mut addr = start & !0xFFF;
    let end = (start + len as u64 + 0xFFF) & !0xFFF;
    while addr < end {
        if mapper.translate_addr(VirtAddr::new(addr)).is_none() {
            return false;
        }
        addr += 0x1000;
    }
    true
}

#[derive(Clone, Copy, Debug)]
pub struct Mapping {
    pub start: u64,
    pub end: u64,
    pub flags: u64,
}

impl Mapping {
    pub fn perms(&self) -> [u8; 4] {
        let mut p = [b'-'; 4];
        p[0] = b'r';
        if self.flags & PAGE_WRITABLE != 0 { p[1] = b'w'; }
        if self.flags & PAGE_NX == 0 { p[2] = b'x'; }
        p[3] = b'p';
        p
    }
}

pub unsafe fn user_mappings_snapshot() -> Vec<Mapping> {
    let mut out: Vec<Mapping> = Vec::new();
    let phys_offset_u64 = PHYS_OFFSET.load(Ordering::Acquire);
    if phys_offset_u64 == 0 { return out; }
    let phys_offset = VirtAddr::new(phys_offset_u64);

    let (pml4_frame, _) = Cr3::read();
    let pml4_virt = phys_offset + pml4_frame.start_address().as_u64();
    let pml4 = pml4_virt.as_ptr::<u64>();

    let entry0 = *pml4.add(USER_PML4_INDEX);
    if entry0 & PAGE_PRESENT == 0 || entry0 & PAGE_USER == 0 { return out; }
    let pdpt = (phys_offset + (entry0 & ADDR_MASK)).as_ptr::<u64>();

    for i0 in 0..512usize {
        let e_pdpt = *pdpt.add(i0);
        if e_pdpt & PAGE_PRESENT == 0 || e_pdpt & PAGE_USER == 0 { continue; }
        let pd = (phys_offset + (e_pdpt & ADDR_MASK)).as_ptr::<u64>();

        for i1 in 0..512usize {
            let e_pd = *pd.add(i1);
            if e_pd & PAGE_PRESENT == 0 || e_pd & PAGE_USER == 0 { continue; }
            let base_virt = ((i0 as u64) << 30) | ((i1 as u64) << 21);
            if e_pd & PDE_HUGE != 0 {
                push_or_extend(&mut out, base_virt, base_virt + 0x20_0000, e_pd);
                continue;
            }
            let pt = (phys_offset + (e_pd & ADDR_MASK)).as_ptr::<u64>();
            for i2 in 0..512usize {
                let e_pt = *pt.add(i2);
                if e_pt & PAGE_PRESENT == 0 || e_pt & PAGE_USER == 0 { continue; }
                let start = base_virt | ((i2 as u64) << 12);
                push_or_extend(&mut out, start, start + 0x1000, e_pt);
            }
        }
    }
    out
}

fn push_or_extend(out: &mut Vec<Mapping>, start: u64, end: u64, e: u64) {
    let norm = e & (PAGE_PRESENT | PAGE_WRITABLE | PAGE_NX | PAGE_USER);
    if let Some(last) = out.last_mut() {
        if last.end == start {
            let last_norm = last.flags & (PAGE_PRESENT | PAGE_WRITABLE | PAGE_NX | PAGE_USER);
            if last_norm == norm { last.end = end; return; }
        }
    }
    out.push(Mapping { start, end, flags: norm });
}

pub unsafe fn map_current_as(
    phys_offset: VirtAddr,
    virt: u64,
    frame: PhysFrame<Size4KiB>,
    flags: PageTableFlags,
    alloc: &mut impl FrameAllocator<Size4KiB>,
) -> bool {
    use x86_64::structures::paging::{Mapper, Page};
    let l4 = active_level_4_table(phys_offset);
    let mut mapper = OffsetPageTable::new(l4, phys_offset);
    let page = Page::<Size4KiB>::containing_address(VirtAddr::new(virt));
    match mapper.map_to(page, frame, flags, alloc) {
        Ok(flush) => { flush.flush(); true }
        Err(_) => false,
    }
}

// ---------- COW refcounting ----------

/// Зафиксировать, что фрейм теперь используется двумя AS.
pub fn cow_mark_shared(phys: u64) {
    *COW_REFS.lock().entry(phys).or_insert(1) += 1;
}

/// Уменьшить refcount. Возвращает `true`, если фрейм можно освобождать
/// (refcount стал 0 либо запись отсутствовала — фрейм не разделялся).
pub fn cow_release(phys: u64) -> bool {
    let mut refs = COW_REFS.lock();
    match refs.get_mut(&phys) {
        Some(n) => {
            if *n <= 1 { refs.remove(&phys); true }
            else { *n -= 1; false }
        }
        None => true,
    }
}

// ---------- Клонирование AS с COW ----------

/// Форк AS: копирует промежуточные таблицы (PDPT/PD/PT) для ребёнка,
/// но **физически не копирует** страницы данных. Вместо этого помечает
/// каждую листовую страницу как read-only + COW в обоих AS и увеличивает
/// refcount. Реальное копирование произойдёт на write-fault.
///
/// Требует, чтобы CR3 указывал на parent AS (мы модифицируем его PTE).
pub unsafe fn clone_user_pages_cow(
    parent_pml4_phys: u64,
    child_pml4_phys: u64,
    phys_offset: VirtAddr,
    alloc: &mut BootInfoFrameAllocator,
) -> bool {
    let parent_pml4 = (phys_offset + parent_pml4_phys).as_ptr::<u64>();
    let child_pml4 = (phys_offset + child_pml4_phys).as_mut_ptr::<u64>();

    let e0 = *parent_pml4.add(USER_PML4_INDEX);
    if e0 & PAGE_PRESENT == 0 || e0 & PAGE_USER == 0 { return true; }

    let Some(new_pdpt_frame) = alloc.allocate_frame() else { return false };
    let new_pdpt_phys = new_pdpt_frame.start_address().as_u64();
    core::ptr::write_bytes((phys_offset + new_pdpt_phys).as_mut_ptr::<u8>(), 0, 4096);
    *child_pml4.add(USER_PML4_INDEX) = new_pdpt_phys | (e0 & 0xFFF) | 0x3;

    let parent_pdpt = (phys_offset + (e0 & ADDR_MASK)).as_ptr::<u64>();
    let child_pdpt = (phys_offset + new_pdpt_phys).as_mut_ptr::<u64>();

    for i3 in 0..512usize {
        let e3 = *parent_pdpt.add(i3);
        if e3 & PAGE_PRESENT == 0 || e3 & PAGE_USER == 0 { continue; }

        let Some(new_pd_frame) = alloc.allocate_frame() else { return false };
        let new_pd_phys = new_pd_frame.start_address().as_u64();
        core::ptr::write_bytes((phys_offset + new_pd_phys).as_mut_ptr::<u8>(), 0, 4096);
        *child_pdpt.add(i3) = new_pd_phys | (e3 & 0xFFF) | 0x3;

        let parent_pd = (phys_offset + (e3 & ADDR_MASK)).as_ptr::<u64>();
        let child_pd = (phys_offset + new_pd_phys).as_mut_ptr::<u64>();

        for i2 in 0..512usize {
            let e2 = *parent_pd.add(i2);
            if e2 & PAGE_PRESENT == 0 || e2 & PAGE_USER == 0 { continue; }
            if e2 & PDE_HUGE != 0 { return false; }

            let Some(new_pt_frame) = alloc.allocate_frame() else { return false };
            let new_pt_phys = new_pt_frame.start_address().as_u64();
            core::ptr::write_bytes((phys_offset + new_pt_phys).as_mut_ptr::<u8>(), 0, 4096);
            *child_pd.add(i2) = new_pt_phys | (e2 & 0xFFF) | 0x3;

            let parent_pt = (phys_offset + (e2 & ADDR_MASK)).as_mut_ptr::<u64>();
            let child_pt = (phys_offset + new_pt_phys).as_mut_ptr::<u64>();

            for i1 in 0..512usize {
                let e1 = *parent_pt.add(i1);
                if e1 & PAGE_PRESENT == 0 || e1 & PAGE_USER == 0 { continue; }

                let leaf_phys = e1 & ADDR_MASK;
                cow_mark_shared(leaf_phys);

                let new_flags = (e1 & !PAGE_WRITABLE) | PTE_COW_BIT;
                *parent_pt.add(i1) = new_flags;
                *child_pt.add(i1) = new_flags;
            }
        }
    }

    // Форсируем flush TLB — иначе CPU может использовать закешированные
    // writable-записи для старого маппинга.
    let (frame, flags) = Cr3::read();
    Cr3::write(frame, flags);
    true
}

/// Обработать write-fault на COW-странице. Копирует фрейм, обновляет PTE
/// текущего AS. Возвращает `true`, если fault обработан — handler должен
/// сделать обычный return, и CPU повторит инструкцию.
pub unsafe fn handle_cow_write(vaddr: u64) -> bool {
    let phys_offset_u64 = PHYS_OFFSET.load(Ordering::Acquire);
    if phys_offset_u64 == 0 { return false; }
    let phys_offset = VirtAddr::new(phys_offset_u64);

    if vaddr >= USER_ADDR_MAX { return false; }

    let (pml4_frame, _) = Cr3::read();
    let pml4 = (phys_offset + pml4_frame.start_address().as_u64()).as_ptr::<u64>();
    let e0 = *pml4.add(USER_PML4_INDEX);
    if e0 & PAGE_PRESENT == 0 { return false; }

    let pdpt = (phys_offset + (e0 & ADDR_MASK)).as_ptr::<u64>();
    let i3 = ((vaddr >> 30) & 0x1FF) as usize;
    let e3 = *pdpt.add(i3);
    if e3 & PAGE_PRESENT == 0 { return false; }

    let pd = (phys_offset + (e3 & ADDR_MASK)).as_ptr::<u64>();
    let i2 = ((vaddr >> 21) & 0x1FF) as usize;
    let e2 = *pd.add(i2);
    if e2 & PAGE_PRESENT == 0 { return false; }
    if e2 & PDE_HUGE != 0 { return false; }

    let pt = (phys_offset + (e2 & ADDR_MASK)).as_mut_ptr::<u64>();
    let i1 = ((vaddr >> 12) & 0x1FF) as usize;
    let e1 = *pt.add(i1);
    if e1 & PAGE_PRESENT == 0 { return false; }
    if e1 & PTE_COW_BIT == 0 { return false; }

    let old_phys = e1 & ADDR_MASK;

    let mut guard = FRAME_ALLOCATOR.lock();
    let alloc = match guard.as_mut() { Some(a) => a, None => return false };
    let Some(new_frame) = alloc.allocate_frame() else { return false };
    let new_phys = new_frame.start_address().as_u64();

    core::ptr::copy_nonoverlapping(
        (phys_offset + old_phys).as_ptr::<u8>(),
        (phys_offset + new_phys).as_mut_ptr::<u8>(),
        4096,
    );

    let flags_only = (e1 & 0xFFF) & !PTE_COW_BIT;
    *pt.add(i1) = new_phys | flags_only | PAGE_WRITABLE;

    if cow_release(old_phys) {
        FrameDeallocator::deallocate_frame(
            alloc,
            PhysFrame::<Size4KiB>::containing_address(PhysAddr::new(old_phys)),
        );
    }
    drop(guard);

    tlb::flush(VirtAddr::new(vaddr & !0xFFF));
    true
}

// ---------- Fallback: eager clone (без COW) ----------

pub unsafe fn clone_user_pages(
    parent_pml4_phys: u64,
    child_pml4_phys: u64,
    phys_offset: VirtAddr,
    alloc: &mut BootInfoFrameAllocator,
) -> bool {
    let parent_pml4 = (phys_offset + parent_pml4_phys).as_ptr::<u64>();
    let child_pml4 = (phys_offset + child_pml4_phys).as_mut_ptr::<u64>();

    let e0 = *parent_pml4.add(USER_PML4_INDEX);
    if e0 & PAGE_PRESENT == 0 || e0 & PAGE_USER == 0 { return true; }

    let Some(new_pdpt_frame) = alloc.allocate_frame() else { return false };
    let new_pdpt_phys = new_pdpt_frame.start_address().as_u64();
    core::ptr::write_bytes((phys_offset + new_pdpt_phys).as_mut_ptr::<u8>(), 0, 4096);
    *child_pml4.add(USER_PML4_INDEX) = new_pdpt_phys | (e0 & 0xFFF) | 0x3;

    let parent_pdpt = (phys_offset + (e0 & ADDR_MASK)).as_ptr::<u64>();
    let child_pdpt = (phys_offset + new_pdpt_phys).as_mut_ptr::<u64>();

    for i3 in 0..512usize {
        let e3 = *parent_pdpt.add(i3);
        if e3 & PAGE_PRESENT == 0 || e3 & PAGE_USER == 0 { continue; }
        let Some(new_pd_frame) = alloc.allocate_frame() else { return false };
        let new_pd_phys = new_pd_frame.start_address().as_u64();
        core::ptr::write_bytes((phys_offset + new_pd_phys).as_mut_ptr::<u8>(), 0, 4096);
        *child_pdpt.add(i3) = new_pd_phys | (e3 & 0xFFF) | 0x3;

        let parent_pd = (phys_offset + (e3 & ADDR_MASK)).as_ptr::<u64>();
        let child_pd = (phys_offset + new_pd_phys).as_mut_ptr::<u64>();
        for i2 in 0..512usize {
            let e2 = *parent_pd.add(i2);
            if e2 & PAGE_PRESENT == 0 || e2 & PAGE_USER == 0 { continue; }
            if e2 & PDE_HUGE != 0 { return false; }

            let Some(new_pt_frame) = alloc.allocate_frame() else { return false };
            let new_pt_phys = new_pt_frame.start_address().as_u64();
            core::ptr::write_bytes((phys_offset + new_pt_phys).as_mut_ptr::<u8>(), 0, 4096);
            *child_pd.add(i2) = new_pt_phys | (e2 & 0xFFF) | 0x3;

            let parent_pt = (phys_offset + (e2 & ADDR_MASK)).as_ptr::<u64>();
            let child_pt = (phys_offset + new_pt_phys).as_mut_ptr::<u64>();
            for i1 in 0..512usize {
                let e1 = *parent_pt.add(i1);
                if e1 & PAGE_PRESENT == 0 || e1 & PAGE_USER == 0 { continue; }
                let Some(new_page_frame) = alloc.allocate_frame() else { return false };
                let new_page_phys = new_page_frame.start_address().as_u64();
                core::ptr::copy_nonoverlapping(
                    (phys_offset + (e1 & ADDR_MASK)).as_ptr::<u8>(),
                    (phys_offset + new_page_phys).as_mut_ptr::<u8>(),
                    4096,
                );
                *child_pt.add(i1) = new_page_phys | (e1 & 0xFFF);
            }
        }
    }
    true
}

// ---------- AddressSpace ----------

pub struct AddressSpace {
    pub pml4_frame: PhysFrame<Size4KiB>,
}

impl AddressSpace {
    pub unsafe fn new_user(
        phys_offset: VirtAddr,
        alloc: &mut impl FrameAllocator<Size4KiB>,
    ) -> Option<Self> {
        let frame = alloc.allocate_frame()?;
        let new_virt = phys_offset + frame.start_address().as_u64();
        let new_pml4 = new_virt.as_mut_ptr::<u64>();
        core::ptr::write_bytes(new_pml4 as *mut u8, 0, 4096);

        let kernel_phys = KERNEL_PML4_PHYS.load(Ordering::Acquire);
        if kernel_phys == 0 { return None; }
        let kernel_pml4 = (phys_offset + kernel_phys).as_ptr::<u64>();

        for i in 0..512 {
            if i == USER_PML4_INDEX { continue; }
            *new_pml4.add(i) = *kernel_pml4.add(i);
        }
        Some(Self { pml4_frame: frame })
    }

    pub unsafe fn mapper(&self, phys_offset: VirtAddr) -> OffsetPageTable<'static> {
        let virt = phys_offset + self.pml4_frame.start_address().as_u64();
        let pml4_ptr: *mut PageTable = virt.as_mut_ptr();
        OffsetPageTable::new(&mut *pml4_ptr, phys_offset)
    }

    pub unsafe fn destroy(pml4_phys: u64) {
        let phys_offset = PHYS_OFFSET.load(Ordering::Acquire);
        if phys_offset == 0 { return; }
        let mut alloc_guard = FRAME_ALLOCATOR.lock();
        let alloc = match alloc_guard.as_mut() { Some(a) => a, None => return };
        let pml4 = (phys_offset + pml4_phys) as *const u64;
        let entry0 = *pml4.add(USER_PML4_INDEX);
        if entry0 & 1 != 0 && entry0 & PAGE_USER != 0 {
            free_pdpt(alloc, phys_offset, entry0 & ADDR_MASK);
        }
        dealloc_frame(alloc, pml4_phys);
    }
}

unsafe fn free_pdpt(alloc: &mut BootInfoFrameAllocator, phys_offset: u64, pdpt_phys: u64) {
    let pdpt = (phys_offset + pdpt_phys) as *const u64;
    for i in 0..512 {
        let e = *pdpt.add(i);
        if e & 1 == 0 || e & PAGE_USER == 0 { continue; }
        free_pd(alloc, phys_offset, e & ADDR_MASK);
    }
    dealloc_frame(alloc, pdpt_phys);
}

unsafe fn free_pd(alloc: &mut BootInfoFrameAllocator, phys_offset: u64, pd_phys: u64) {
    let pd = (phys_offset + pd_phys) as *const u64;
    for i in 0..512 {
        let e = *pd.add(i);
        if e & 1 == 0 || e & PAGE_USER == 0 { continue; }
        if e & PDE_HUGE != 0 {
            let page_phys = e & HUGE_ADDR_MASK;
            if cow_release(page_phys) { dealloc_frame(alloc, page_phys); }
            continue;
        }
        free_pt(alloc, phys_offset, e & ADDR_MASK);
    }
    dealloc_frame(alloc, pd_phys);
}

unsafe fn free_pt(alloc: &mut BootInfoFrameAllocator, phys_offset: u64, pt_phys: u64) {
    let pt = (phys_offset + pt_phys) as *const u64;
    for i in 0..512 {
        let e = *pt.add(i);
        if e & 1 == 0 || e & PAGE_USER == 0 { continue; }
        let page_phys = e & ADDR_MASK;
        // С COW фрейм может использоваться несколькими AS.
        if cow_release(page_phys) {
            dealloc_frame(alloc, page_phys);
        }
    }
    dealloc_frame(alloc, pt_phys);
}

unsafe fn dealloc_frame(alloc: &mut BootInfoFrameAllocator, phys: u64) {
    let frame = PhysFrame::<Size4KiB>::containing_address(PhysAddr::new(phys));
    FrameDeallocator::deallocate_frame(alloc, frame);
}

// ---------- Frame allocator ----------

pub struct BootInfoFrameAllocator {
    memory_regions: &'static MemoryRegions,
    next_region: usize,
    next_addr: u64,
    freed: BTreeSet<PhysFrame<Size4KiB>>,
}

impl BootInfoFrameAllocator {
    pub unsafe fn init(memory_regions: &'static MemoryRegions) -> Self {
        let mut a = Self {
            memory_regions,
            next_region: 0,
            next_addr: 0,
            freed: BTreeSet::new(),
        };
        a.skip_to_next_usable();
        crate::serial_println!(
            "[mem] allocator init: next_region = {}, next_addr = {:#x}",
            a.next_region, a.next_addr
        );
        a
    }
    fn region_count(&self) -> usize { self.memory_regions.iter().count() }
    fn skip_to_next_usable(&mut self) {
        let count = self.region_count();
        while self.next_region < count {
            let r = match self.memory_regions.iter().nth(self.next_region) {
                Some(r) => r,
                None => { self.next_region = count; return; }
            };
            if r.kind == MemoryRegionKind::Usable {
                let start = r.start.max(LOW_MEM_LIMIT);
                let aligned = (start + 0xFFF) & !0xFFF;
                if aligned + 4096 <= r.end { self.next_addr = aligned; return; }
            }
            self.next_region += 1;
        }
    }
}

unsafe impl FrameAllocator<Size4KiB> for BootInfoFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        if let Some(&f) = self.freed.iter().next() { self.freed.remove(&f); return Some(f); }
        loop {
            if self.next_region >= self.region_count() { return None; }
            let r = self.memory_regions.iter().nth(self.next_region)?;
            if self.next_addr + 4096 <= r.end {
                let addr = self.next_addr;
                self.next_addr += 4096;
                return Some(PhysFrame::containing_address(PhysAddr::new(addr)));
            }
            self.next_region += 1;
            self.skip_to_next_usable();
        }
    }
}

impl FrameDeallocator<Size4KiB> for BootInfoFrameAllocator {
    unsafe fn deallocate_frame(&mut self, frame: PhysFrame<Size4KiB>) {
        if !self.freed.insert(frame) {
            crate::log_warn!(
                "[mem] double free frame {:#x} — игнорирую",
                frame.start_address().as_u64()
            );
        }
    }
}