//! Минимальный загрузчик ELF64 (little-endian, ET_EXEC или ET_DYN).
//!
//! Загружает PT_LOAD-сегменты в текущую page table с флагами
//! PRESENT|WRITABLE|USER_ACCESSIBLE. Executable-бит в long mode не
//! различается по умолчанию (NX не включён).

use alloc::vec::Vec;

use x86_64::structures::paging::{
    FrameAllocator, Mapper, Page, PageTableFlags, Size4KiB,
};
use x86_64::VirtAddr;

#[derive(Debug)]
pub enum ElfError {
    BadMagic,
    Not64,
    NotLittleEndian,
    NotExec,
    BadOffset,
    MapFailed,
}

pub struct LoadedElf {
    pub entry: u64,
    /// (vaddr_start, vaddr_end, flags) для каждого загруженного сегмента.
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
    // 2 = ET_EXEC, 3 = ET_DYN. Принимаем оба — наш linker.ld задаёт
    // абсолютную базу 0x400000, так что PIE-адреса уже абсолютные.
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

        let _p_flags = u32::from_le_bytes(ph[4..8].try_into().unwrap());
        let p_offset = u64::from_le_bytes(ph[8..16].try_into().unwrap()) as usize;
        let p_vaddr = u64::from_le_bytes(ph[16..24].try_into().unwrap());
        let p_filesz = u64::from_le_bytes(ph[32..40].try_into().unwrap()) as usize;
        let p_memsz = u64::from_le_bytes(ph[40..48].try_into().unwrap()) as usize;

        if p_memsz == 0 {
            continue;
        }

        let flags = PageTableFlags::PRESENT
            | PageTableFlags::WRITABLE
            | PageTableFlags::USER_ACCESSIBLE;

        let vstart = p_vaddr & !0xFFF;
        let vend = (p_vaddr + p_memsz as u64 + 0xFFF) & !0xFFF;

        let start_page = Page::containing_address(VirtAddr::new(vstart));
        let end_page = Page::containing_address(VirtAddr::new(vend - 1));

        for page in Page::range_inclusive(start_page, end_page) {
            let frame = alloc.allocate_frame().ok_or(ElfError::MapFailed)?;
            unsafe {
                mapper
                    .map_to(page, frame, flags, alloc)
                    .map_err(|_| ElfError::MapFailed)?
                    .flush();
            }
        }

        // Копируем содержимое.
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

        // Обнуляем хвост memsz (bss).
        let remaining = p_memsz - p_filesz;
        if remaining > 0 {
            unsafe {
                core::ptr::write_bytes((p_vaddr + p_filesz as u64) as *mut u8, 0, remaining);
            }
        }

        segments.push((vstart, vend, flags));
    }

    Ok(LoadedElf { entry: e_entry, segments })
}