//! Минимальный загрузчик ELF64 (little-endian, ET_EXEC или ET_DYN).
//!
//! Загружает PT_LOAD-сегменты в текущую page table.
//!
//! # W^X и NX
//!
//! Флаги страниц берутся из `p_flags` ELF-сегмента:
//! - `R+X` → `PRESENT | USER` (RX)
//! - `R+W` → `PRESENT | WRITABLE | USER | NO_EXECUTE` (RW+NX)
//! - `R`   → `PRESENT | USER | NO_EXECUTE` (R, только чтение)
//!
//! # Двухфазный маппинг
//!
//! С CR0.WP=1 нельзя писать в read-only страницу даже из Ring 0.
//! Поэтому мапим сначала **все** страницы сегмента как `RW`, заливаем
//! содержимое, и только потом через `update_flags` ставим финальные
//! флаги. Это единственный безопасный способ разложить .text-сегмент
//! в свежий AS.
//!
//! Порядок бит `p_flags` (стандарт ELF):
//! - бит 0 (1) = X (execute)
//! - бит 1 (2) = W (write)
//! - бит 2 (4) = R (read)

use alloc::vec::Vec;

use x86_64::structures::paging::{
    FrameAllocator, Mapper, Page, PageTableFlags, Size4KiB,
};
use x86_64::VirtAddr;

use crate::{log_debug, log_warn};

#[derive(Debug)]
pub enum ElfError {
    BadMagic,
    Not64,
    NotLittleEndian,
    NotExec,
    BadOffset,
    MapFailed,
    EntryOutsideSegments,
}

pub struct LoadedElf {
    pub entry: u64,
    pub segments: Vec<(u64, u64, PageTableFlags)>,
}

pub fn load(
    data: &[u8],
    mapper: &mut impl Mapper<Size4KiB>,
    alloc: &mut impl FrameAllocator<Size4KiB>,
) -> Result<LoadedElf, ElfError> {
    if data.len() < 64 {
        return Err(ElfError::BadOffset);
    }
    if &data[0..4] != b"\x7FELF" {
        return Err(ElfError::BadMagic);
    }
    if data[4] != 2 {
        return Err(ElfError::Not64);
    }
    if data[5] != 1 {
        return Err(ElfError::NotLittleEndian);
    }

    let e_type = u16::from_le_bytes([data[16], data[17]]);
    if e_type != 2 && e_type != 3 {
        return Err(ElfError::NotExec);
    }

    let e_entry = u64::from_le_bytes(data[24..32].try_into().unwrap());
    let e_phoff = u64::from_le_bytes(data[32..40].try_into().unwrap()) as usize;
    let e_phentsize = u16::from_le_bytes([data[54], data[55]]) as usize;
    let e_phnum = u16::from_le_bytes([data[56], data[57]]) as usize;

    if e_phentsize < 56 {
        return Err(ElfError::BadOffset);
    }

    let mut segments = Vec::new();

    for i in 0..e_phnum {
        let ph_off = e_phoff + i * e_phentsize;
        if ph_off + 56 > data.len() {
            return Err(ElfError::BadOffset);
        }
        let ph = &data[ph_off..ph_off + 56];

        let p_type = u32::from_le_bytes(ph[0..4].try_into().unwrap());
        if p_type != 1 {
            continue;
        }

        let p_flags = u32::from_le_bytes(ph[4..8].try_into().unwrap());
        let p_offset = u64::from_le_bytes(ph[8..16].try_into().unwrap()) as usize;
        let p_vaddr = u64::from_le_bytes(ph[16..24].try_into().unwrap());
        let p_filesz = u64::from_le_bytes(ph[32..40].try_into().unwrap()) as usize;
        let p_memsz = u64::from_le_bytes(ph[40..48].try_into().unwrap()) as usize;

        if p_memsz == 0 {
            continue;
        }

        // ELF p_flags: bit0 = X, bit1 = W, bit2 = R.
        let executable = p_flags & 0b001 != 0;
        let writable   = p_flags & 0b010 != 0;

        // Финальные флаги (что хотим получить в итоге).
        let mut final_flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
        if writable {
            final_flags.insert(PageTableFlags::WRITABLE);
        }
        if !executable {
            final_flags.insert(PageTableFlags::NO_EXECUTE);
        }

        // Временные флаги: RW+USER, чтобы можно было записать содержимое
        // без #PF. NX не ставим — на запись он не влияет, но при отладке
        // проще видеть ровные права на время загрузки.
        let temp_flags = PageTableFlags::PRESENT
            | PageTableFlags::WRITABLE
            | PageTableFlags::USER_ACCESSIBLE;

        let vstart = p_vaddr & !0xFFF;
        let vend = (p_vaddr + p_memsz as u64 + 0xFFF) & !0xFFF;

        let start_page = Page::containing_address(VirtAddr::new(vstart));
        let end_page = Page::containing_address(VirtAddr::new(vend - 1));

        // Фаза 1: маппим всё как RW.
        for page in Page::range_inclusive(start_page, end_page) {
            let frame = alloc.allocate_frame().ok_or(ElfError::MapFailed)?;
            unsafe {
                mapper
                    .map_to(page, frame, temp_flags, alloc)
                    .map_err(|_| ElfError::MapFailed)?
                    .flush();
            }
        }

        // Фаза 2: заливаем содержимое.
        if p_filesz > 0 {
            if p_offset + p_filesz > data.len() {
                return Err(ElfError::BadOffset);
            }
            unsafe {
                core::ptr::copy_nonoverlapping(
                    data.as_ptr().add(p_offset),
                    p_vaddr as *mut u8,
                    p_filesz,
                );
            }
        }

        let remaining = p_memsz - p_filesz;
        if remaining > 0 {
            unsafe {
                core::ptr::write_bytes((p_vaddr + p_filesz as u64) as *mut u8, 0, remaining);
            }
        }

        // Фаза 3: применяем финальные флаги.
        //
        // update_flags работает только с 4-KiB страницами; у нас все
        // страницы 4-KiB, поэтому конфликтов не будет.
        for page in Page::range_inclusive(start_page, end_page) {
            unsafe {
                mapper
                    .update_flags(page, final_flags)
                    .map_err(|_| ElfError::MapFailed)?
                    .flush();
            }
        }

        segments.push((vstart, vend, final_flags));
    }

    let mut entry_ok = false;
    for (vstart, vend, _) in &segments {
        if e_entry >= *vstart && e_entry < *vend {
            entry_ok = true;
            break;
        }
    }
    if !entry_ok {
        log_warn!(
            "[elf] entry {:#x} не входит в сегменты — битый ELF",
            e_entry
        );
        return Err(ElfError::EntryOutsideSegments);
    }

    log_debug!("[elf] loaded, entry={:#x}, segments={}", e_entry, segments.len());
    Ok(LoadedElf { entry: e_entry, segments })
}