use bootloader_api::info::{MemoryRegionKind, MemoryRegions};
use x86_64::{
    structures::paging::{
        FrameAllocator, OffsetPageTable, PageTable, PhysFrame, Size4KiB,
    },
    PhysAddr, VirtAddr,
};

/// Первый мегабайт физической памяти трогать нельзя — там IVT, BDA, VGA и т.д.
const LOW_MEM_LIMIT: u64 = 0x10_0000;

/// Инициализирует новый `OffsetPageTable`.
///
/// # Безопасность
/// Вызывающий должен гарантировать, что `physical_memory_offset` корректен
/// и вся физическая память отображена на этот виртуальный адрес.
pub unsafe fn init(physical_memory_offset: VirtAddr) -> OffsetPageTable<'static> {
    let level_4_table = active_level_4_table(physical_memory_offset);
    OffsetPageTable::new(level_4_table, physical_memory_offset)
}

unsafe fn active_level_4_table(physical_memory_offset: VirtAddr) -> &'static mut PageTable {
    use x86_64::registers::control::Cr3;

    let (level_4_table_frame, _) = Cr3::read();
    let phys = level_4_table_frame.start_address();
    let virt = physical_memory_offset + phys.as_u64();
    let page_table_ptr: *mut PageTable = virt.as_mut_ptr();

    &mut *page_table_ptr
}

/// Аллокатор физических фреймов с O(1) выделением.
pub struct BootInfoFrameAllocator {
    memory_regions: &'static MemoryRegions,
    next_region: usize,
    next_addr: u64,
}

impl BootInfoFrameAllocator {
    /// Создаёт аллокатор из карты памяти загрузчика.
    ///
    /// # Безопасность
    /// Фреймы, помеченные как `Usable`, действительно не должны использоваться.
    pub unsafe fn init(memory_regions: &'static MemoryRegions) -> Self {
        let mut a = Self {
            memory_regions,
            next_region: 0,
            next_addr: 0,
        };
        a.skip_to_next_usable();
        crate::serial_println!(
            "[mem] allocator init: next_region = {}, next_addr = {:#x}",
            a.next_region,
            a.next_addr
        );
        a
    }

    fn region_count(&self) -> usize {
        self.memory_regions.iter().count()
    }

    /// Ищем следующий usable-регион, начиная с `next_region`, и ставим
    /// `next_addr` на его начало (не ниже LOW_MEM_LIMIT), выровненное на 4096.
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
        loop {
            if self.next_region >= self.region_count() {
                return None;
            }
            let r = self.memory_regions.iter().nth(self.next_region)?;
            // Если следующий фрейм умещается в текущем регионе — отдаём его.
            if self.next_addr + 4096 <= r.end {
                let addr = self.next_addr;
                self.next_addr += 4096;
                return Some(PhysFrame::containing_address(PhysAddr::new(addr)));
            }
            // Иначе — следующий регион.
            self.next_region += 1;
            self.skip_to_next_usable();
        }
    }
}