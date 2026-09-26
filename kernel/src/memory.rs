use bootloader_api::info::{MemoryRegionKind, MemoryRegions};
use core::sync::atomic::{AtomicU64, Ordering};
use spin::Mutex;
use x86_64::{
    registers::control::Cr3,
    structures::paging::{
        FrameAllocator, FrameDeallocator, OffsetPageTable, PageTable, PageTableFlags,
        PhysFrame, Size4KiB,
    },
    PhysAddr, VirtAddr,
};

const LOW_MEM_LIMIT: u64 = 0x10_0000;

pub const USER_PML4_INDEX: usize = 0;

/// Бит 2 в PTE/PDE/PDPTE: USER_ACCESSIBLE.
const PAGE_USER: u64 = 1 << 2;
/// Бит 7 в PDE: 2 MiB huge page.
const PDE_HUGE: u64 = 1 << 7;
/// Маска физического адреса для 4 KiB фрейма.
const ADDR_MASK: u64 = 0x000F_FFFF_FFFF_F000;
/// Маска для 2 MiB huge page.
const HUGE_ADDR_MASK: u64 = 0x000F_FFFF_FFE0_0000;

pub static FRAME_ALLOCATOR: Mutex<Option<BootInfoFrameAllocator>> = Mutex::new(None);
pub static KERNEL_PML4_PHYS: AtomicU64 = AtomicU64::new(0);
pub static PHYS_OFFSET: AtomicU64 = AtomicU64::new(0);

pub unsafe fn init(physical_memory_offset: VirtAddr) -> OffsetPageTable<'static> {
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

    /// Освобождает user-ветку (PML4[USER_PML4_INDEX]) и сам PML4.
    ///
    /// # Safety
    /// Вызывается только когда этот AS больше никем не используется.
    /// Мы никогда не трогаем kernel-shared entries (проверяем USER_ACCESSIBLE).
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

        // PML4[0] должен существовать и быть user-accessible.
        if entry0 & 1 != 0 && entry0 & PAGE_USER != 0 {
            let pdpt_phys = entry0 & ADDR_MASK;
            free_pdpt(alloc, phys_offset, pdpt_phys);
        }
        // В любом случае освобождаем саму PML4.
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
            // 2 MiB huge page — освобождаем один фрейм.
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
    freed: alloc::vec::Vec<PhysFrame<Size4KiB>>,
}

impl BootInfoFrameAllocator {
    pub unsafe fn init(memory_regions: &'static MemoryRegions) -> Self {
        let mut a = Self {
            memory_regions,
            next_region: 0,
            next_addr: 0,
            freed: alloc::vec::Vec::new(),
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
        if let Some(f) = self.freed.pop() {
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
        self.freed.push(frame);
    }
}