use alloc::collections::BTreeSet;
use alloc::vec::Vec;
use bootloader_api::info::{MemoryRegionKind, MemoryRegions};
use core::sync::atomic::{AtomicU64, Ordering};
use spin::Mutex;
use x86_64::{
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

/// Бит 63 в PTE: NX (no-execute). 1 = страница не исполняема.
const PAGE_NX: u64 = 1 << 63;
/// Бит 1 в PTE: WRITABLE.
const PAGE_WRITABLE: u64 = 1 << 1;
/// Бит 0 в PTE: PRESENT.
const PAGE_PRESENT: u64 = 1 << 0;

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

/// Одна запись карты памяти.
#[derive(Clone, Copy, Debug)]
pub struct Mapping {
    pub start: u64,
    pub end: u64,
    pub flags: u64,
}

impl Mapping {
    /// Разрешение в формате `/proc/self/maps` (`rwx` + `p`).
    pub fn perms(&self) -> [u8; 4] {
        let mut p = [b'-'; 4];
        // PRESENT всегда у наших записей
        p[0] = b'r';
        if self.flags & PAGE_WRITABLE != 0 {
            p[1] = b'w';
        }
        if self.flags & PAGE_NX == 0 {
            p[2] = b'x';
        }
        p[3] = b'p'; // private, мы не различаем shared/private
        p
    }
}

/// ИСПРАВЛЕНИЕ: снимок всех user-маппингов в текущем AS.
///
/// Обходит PML4[USER_PML4_INDEX] → PDPTE → PD → PT и группирует смежные
/// страницы с одинаковыми флагами в один диапазон.
///
/// Поддерживает 4 KiB PT и 2 MiB PDE-huge (последний — только для
/// полноты; сейчас у нас таких нет, но лучше уметь).
///
/// Возвращает пустой вектор, если PML4[0] не отображён (kernel-only
/// address space — например, до создания user-AS).
///
/// # Safety
/// Читает таблицы страниц по `PHYS_OFFSET` — только из ядра.
pub unsafe fn user_mappings_snapshot() -> Vec<Mapping> {
    let mut out: Vec<Mapping> = Vec::new();
    let phys_offset_u64 = PHYS_OFFSET.load(Ordering::Acquire);
    if phys_offset_u64 == 0 { return out; }
    let phys_offset = VirtAddr::new(phys_offset_u64);

    let (pml4_frame, _) = Cr3::read();
    let pml4_virt = phys_offset + pml4_frame.start_address().as_u64();
    let pml4 = pml4_virt.as_ptr::<u64>();

    let entry0 = *pml4.add(USER_PML4_INDEX);
    if entry0 & PAGE_PRESENT == 0 || entry0 & PAGE_USER == 0 {
        return out;
    }

    let pdpt_phys = entry0 & ADDR_MASK;
    let pdpt = (phys_offset + pdpt_phys).as_ptr::<u64>();

    for i0 in 0..512usize {
        let e_pdpt = *pdpt.add(i0);
        if e_pdpt & PAGE_PRESENT == 0 || e_pdpt & PAGE_USER == 0 {
            continue;
        }
        let pd_phys = e_pdpt & ADDR_MASK;
        let pd = (phys_offset + pd_phys).as_ptr::<u64>();

        for i1 in 0..512usize {
            let e_pd = *pd.add(i1);
            if e_pd & PAGE_PRESENT == 0 || e_pd & PAGE_USER == 0 {
                continue;
            }

            let base_virt = ((i0 as u64) << 30) | ((i1 as u64) << 21);

            if e_pd & PDE_HUGE != 0 {
                // 2 MiB page.
                let start = base_virt;
                let end = base_virt + 0x20_0000;
                push_or_extend(&mut out, start, end, e_pd);
                continue;
            }

            let pt_phys = e_pd & ADDR_MASK;
            let pt = (phys_offset + pt_phys).as_ptr::<u64>();

            for i2 in 0..512usize {
                let e_pt = *pt.add(i2);
                if e_pt & PAGE_PRESENT == 0 || e_pt & PAGE_USER == 0 {
                    continue;
                }
                let start = base_virt | ((i2 as u64) << 12);
                let end = start + 0x1000;
                push_or_extend(&mut out, start, end, e_pt);
            }
        }
    }
    out
}

/// Если новый диапазон смежен с последним и совпадает по флагам (кроме NX
/// и WRITABLE — их маскируем), расширяем последний. Иначе добавляем новый.
fn push_or_extend(out: &mut Vec<Mapping>, start: u64, end: u64, e: u64) {
    let norm = e & (PAGE_PRESENT | PAGE_WRITABLE | PAGE_NX | PAGE_USER);
    if let Some(last) = out.last_mut() {
        if last.end == start {
            let last_norm = last.flags & (PAGE_PRESENT | PAGE_WRITABLE | PAGE_NX | PAGE_USER);
            if last_norm == norm {
                last.end = end;
                return;
            }
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
        Ok(flush) => {
            flush.flush();
            true
        }
        Err(_) => false,
    }
}

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
        if kernel_phys == 0 {
            return None;
        }
        let kernel_virt = phys_offset + kernel_phys;
        let kernel_pml4 = kernel_virt.as_ptr::<u64>();

        for i in 0..512 {
            if i == USER_PML4_INDEX {
                continue;
            }
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
        if phys_offset == 0 {
            return;
        }
        let mut alloc_guard = FRAME_ALLOCATOR.lock();
        let alloc = match alloc_guard.as_mut() {
            Some(a) => a,
            None => return,
        };
        let pml4_virt = phys_offset + pml4_phys;
        let pml4 = pml4_virt as *const u64;
        let entry0 = *pml4.add(USER_PML4_INDEX);

        if entry0 & 1 != 0 && entry0 & PAGE_USER != 0 {
            let pdpt_phys = entry0 & ADDR_MASK;
            free_pdpt(alloc, phys_offset, pdpt_phys);
        }
        dealloc_frame(alloc, pml4_phys);
    }
}

unsafe fn free_pdpt(alloc: &mut BootInfoFrameAllocator, phys_offset: u64, pdpt_phys: u64) {
    let pdpt = (phys_offset + pdpt_phys) as *const u64;
    for i in 0..512 {
        let e = *pdpt.add(i);
        if e & 1 == 0 || e & PAGE_USER == 0 {
            continue;
        }
        let pd_phys = e & ADDR_MASK;
        free_pd(alloc, phys_offset, pd_phys);
    }
    dealloc_frame(alloc, pdpt_phys);
}

unsafe fn free_pd(alloc: &mut BootInfoFrameAllocator, phys_offset: u64, pd_phys: u64) {
    let pd = (phys_offset + pd_phys) as *const u64;
    for i in 0..512 {
        let e = *pd.add(i);
        if e & 1 == 0 || e & PAGE_USER == 0 {
            continue;
        }
        if e & PDE_HUGE != 0 {
            let page_phys = e & HUGE_ADDR_MASK;
            dealloc_frame(alloc, page_phys);
            continue;
        }
        let pt_phys = e & ADDR_MASK;
        free_pt(alloc, phys_offset, pt_phys);
    }
    dealloc_frame(alloc, pd_phys);
}

unsafe fn free_pt(alloc: &mut BootInfoFrameAllocator, phys_offset: u64, pt_phys: u64) {
    let pt = (phys_offset + pt_phys) as *const u64;
    for i in 0..512 {
        let e = *pt.add(i);
        if e & 1 == 0 || e & PAGE_USER == 0 {
            continue;
        }
        let page_phys = e & ADDR_MASK;
        dealloc_frame(alloc, page_phys);
    }
    dealloc_frame(alloc, pt_phys);
}

unsafe fn dealloc_frame(alloc: &mut BootInfoFrameAllocator, phys: u64) {
    let frame = PhysFrame::<Size4KiB>::containing_address(PhysAddr::new(phys));
    FrameDeallocator::deallocate_frame(alloc, frame);
}

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

    fn region_count(&self) -> usize {
        self.memory_regions.iter().count()
    }

    fn skip_to_next_usable(&mut self) {
        let count = self.region_count();
        while self.next_region < count {
            let r = match self.memory_regions.iter().nth(self.next_region) {
                Some(r) => r,
                None => {
                    self.next_region = count;
                    return;
                }
            };
            if r.kind == MemoryRegionKind::Usable {
                let start = r.start.max(LOW_MEM_LIMIT);
                let aligned = (start + 0xFFF) & !0xFFF;
                if aligned + 4096 <= r.end {
                    self.next_addr = aligned;
                    return;
                }
            }
            self.next_region += 1;
        }
    }
}

unsafe impl FrameAllocator<Size4KiB> for BootInfoFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        if let Some(&f) = self.freed.iter().next() {
            self.freed.remove(&f);
            return Some(f);
        }
        loop {
            if self.next_region >= self.region_count() {
                return None;
            }
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