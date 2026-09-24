use alloc::alloc::Layout;
use bootloader_api::info::{MemoryRegionKind, MemoryRegions};
use linked_list_allocator::LockedHeap;

pub const HEAP_SIZE: usize = 4 * 1024 * 1024;

#[global_allocator]
pub static ALLOCATOR: LockedHeap = LockedHeap::empty();

pub fn init_heap(memory_regions: &MemoryRegions, phys_offset: u64) {
    for region in memory_regions.iter() {
        if region.kind != MemoryRegionKind::Usable {
            continue;
        }
        if region.end - region.start < HEAP_SIZE as u64 {
            continue;
        }
        // Region задан в физических адресах; у нас есть identity-маппинг
        // с offset = physical_memory_offset, поэтому виртуальный = phys + offset.
        let heap_start = (region.start + phys_offset) as *mut u8;
        unsafe {
            ALLOCATOR.lock().init(heap_start, HEAP_SIZE);
        }
        return;
    }
    panic!("Не найден подходящий регион для heap");
}

#[alloc_error_handler]
fn alloc_error(_layout: Layout) -> ! {
    panic!("allocation error");
}