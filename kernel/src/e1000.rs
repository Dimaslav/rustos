//! Intel e1000 (82540EM) — драйвер с RX/TX ring'ами.

use alloc::vec::Vec;
use core::sync::atomic::Ordering;
use spin::Mutex;
use x86_64::structures::paging::FrameAllocator;

pub const MAC_LEN: usize = 6;
pub const ETH_FRAME_MAX: usize = 1518;

const RX_RING_SIZE: usize = 32;
const TX_RING_SIZE: usize = 8;
const RX_BUF_SIZE: usize = 2048;
const TX_BUF_SIZE: usize = 2048;

pub struct E1000 {
    pub bar_phys: u64,
    pub bar_size: u64,
    mmio: *mut u8,
    pub mac: [u8; 6],

    rx_ring_phys: u64,
    rx_ring_virt: *mut u8,
    rx_bufs_phys: u64,
    rx_bufs_virt: *mut u8,
    rx_cur: usize,

    tx_ring_phys: u64,
    tx_ring_virt: *mut u8,
    tx_bufs_phys: u64,
    tx_bufs_virt: *mut u8,
    tx_cur: usize,
}

unsafe impl Send for E1000 {}
unsafe impl Sync for E1000 {}

static E1000: Mutex<Option<E1000>> = Mutex::new(None);

#[inline]
unsafe fn mmio_read32(base: *mut u8, off: u32) -> u32 {
    core::ptr::read_volatile(base.add(off as usize) as *const u32)
}

#[inline]
unsafe fn mmio_write32(base: *mut u8, off: u32, val: u32) {
    core::ptr::write_volatile(base.add(off as usize) as *mut u32, val);
}

// ---------- Регистры ----------

const REG_CTRL:   u32 = 0x0000;
const REG_STATUS: u32 = 0x0008;
const REG_RCTL:   u32 = 0x0100;
const REG_TCTL:   u32 = 0x0400;
const REG_RDBAL:  u32 = 0x2800;
const REG_RDBAH:  u32 = 0x2804;
const REG_RDLEN:  u32 = 0x2808;
const REG_RDH:    u32 = 0x2810;
const REG_RDT:    u32 = 0x2818;
const REG_TDBAL:  u32 = 0x3800;
const REG_TDBAH:  u32 = 0x3804;
const REG_TDLEN:  u32 = 0x3808;
const REG_TDH:    u32 = 0x3810;
const REG_TDT:    u32 = 0x3818;
const REG_RAL:    u32 = 0x5400;
const REG_RAH:    u32 = 0x5404;
const REG_IMC:    u32 = 0x00D8;

const CTRL_RST:  u32 = 1 << 26;
const STATUS_LU: u32 = 1 << 1;

const RCTL_EN:    u32 = 1 << 1;
const RCTL_SBP:   u32 = 1 << 2;
const RCTL_UPE:   u32 = 1 << 3;
const RCTL_MPE:   u32 = 1 << 4;
const RCTL_BAM:   u32 = 1 << 15;
const RCTL_SECRC: u32 = 1 << 26;

const TCTL_EN:   u32 = 1 << 1;
const TCTL_PSP:  u32 = 1 << 3;
const TCTL_CT_SHIFT: u32 = 4;
const TCTL_COLD_SHIFT: u32 = 12;

const TX_CMD_EOP:  u8 = 1 << 0;
const TX_CMD_IFCS: u8 = 1 << 1;
const TX_CMD_RS:   u8 = 1 << 3;
const TX_STATUS_DD: u8 = 1 << 0;

const RX_STATUS_DD: u8 = 1 << 0;

const E1000_DEVICE_IDS: &[u16] = &[
    0x100E, 0x100F, 0x10D3, 0x1004, 0x153A,
];

// ---------- Helpers для дескрипторов ----------

unsafe fn write_u64(dst: *mut u8, v: u64) { core::ptr::write_volatile(dst as *mut u64, v); }
unsafe fn write_u16(dst: *mut u8, v: u16) { core::ptr::write_volatile(dst as *mut u16, v); }
unsafe fn write_u8(dst: *mut u8, v: u8)   { core::ptr::write_volatile(dst, v); }

unsafe fn read_u64(src: *const u8) -> u64 { core::ptr::read_volatile(src as *const u64) }
unsafe fn read_u16(src: *const u8) -> u16 { core::ptr::read_volatile(src as *const u16) }
unsafe fn read_u8(src: *const u8) -> u8   { core::ptr::read_volatile(src) }

/// Аллоцировать N последовательных физических фреймов.
unsafe fn alloc_contig(pages: usize) -> Option<(u64, *mut u8)> {
    let phys_offset = crate::memory::PHYS_OFFSET.load(Ordering::Acquire);
    if phys_offset == 0 { return None; }

    let mut addrs: Vec<u64> = Vec::with_capacity(pages);
    {
        let mut guard = crate::memory::FRAME_ALLOCATOR.lock();
        let alloc = guard.as_mut()?;
        for _ in 0..pages {
            let f = alloc.allocate_frame()?;
            addrs.push(f.start_address().as_u64());
        }
    }
    for i in 1..addrs.len() {
        if addrs[i] != addrs[i - 1] + 4096 {
            crate::log_warn!(
                "[e1000] DMA память не непрерывна: {:#x} → {:#x}",
                addrs[i - 1], addrs[i]
            );
            return None;
        }
    }
    let base_phys = addrs[0];
    let base_virt = (phys_offset + base_phys) as *mut u8;
    core::ptr::write_bytes(base_virt, 0, pages * 4096);
    Some((base_phys, base_virt))
}

pub fn init() -> bool {
    let devices = crate::pci::scan_all();
    let dev = devices.iter().find(|d| {
        d.vendor == 0x8086 && E1000_DEVICE_IDS.contains(&d.device)
    });
    let dev = match dev {
        Some(d) => *d,
        None => {
            crate::log_warn!("[e1000] not found on PCI bus");
            return false;
        }
    };

    crate::log_info!(
        "[e1000] found {:04x}:{:04x} at {:02x}:{:02x}.{}",
        dev.vendor, dev.device, dev.bus, dev.slot, dev.func
    );

    crate::pci::enable_bus_master(&dev);

    let bar0 = dev.bar[0];
    if bar0 & 1 != 0 {
        crate::log_warn!("[e1000] BAR0 is I/O (unexpected)");
        return false;
    }
    let bar_phys = (bar0 & 0xFFFF_FFF0) as u64;
    if bar_phys == 0 {
        crate::log_warn!("[e1000] BAR0 = 0");
        return false;
    }
    let bar_size = crate::pci::bar_size_mmio(&dev, 0);

    let phys_offset = crate::memory::PHYS_OFFSET.load(Ordering::Acquire);
    if phys_offset == 0 {
        crate::log_warn!("[e1000] phys_offset = 0");
        return false;
    }
    let mmio = (phys_offset + bar_phys) as *mut u8;

    unsafe {
        let ctrl = mmio_read32(mmio, REG_CTRL);
        mmio_write32(mmio, REG_CTRL, ctrl | CTRL_RST);
        for _ in 0..1_000_000 { core::hint::spin_loop(); }
    }

    unsafe { mmio_write32(mmio, REG_IMC, 0xFFFF_FFFF); }

    let ral = unsafe { mmio_read32(mmio, REG_RAL) };
    let rah = unsafe { mmio_read32(mmio, REG_RAH) };
    let mac = [
        (ral & 0xFF) as u8,
        ((ral >> 8) & 0xFF) as u8,
        ((ral >> 16) & 0xFF) as u8,
        ((ral >> 24) & 0xFF) as u8,
        (rah & 0xFF) as u8,
        ((rah >> 8) & 0xFF) as u8,
    ];

    let (rx_ring_phys, rx_ring_virt) = match unsafe { alloc_contig(1) } {
        Some(v) => v,
        None => { crate::log_warn!("[e1000] cannot alloc RX ring"); return false; }
    };
    let (tx_ring_phys, tx_ring_virt) = match unsafe { alloc_contig(1) } {
        Some(v) => v,
        None => { crate::log_warn!("[e1000] cannot alloc TX ring"); return false; }
    };
    let (rx_bufs_phys, rx_bufs_virt) = match unsafe { alloc_contig(16) } {
        Some(v) => v,
        None => { crate::log_warn!("[e1000] cannot alloc RX bufs"); return false; }
    };
    let (tx_bufs_phys, tx_bufs_virt) = match unsafe { alloc_contig(4) } {
        Some(v) => v,
        None => { crate::log_warn!("[e1000] cannot alloc TX bufs"); return false; }
    };

    // RX ring
    unsafe {
        for i in 0..RX_RING_SIZE {
            let desc = rx_ring_virt.add(i * 16);
            let buf_phys = rx_bufs_phys + (i * RX_BUF_SIZE) as u64;
            write_u64(desc, buf_phys);
            write_u16(desc.add(8), 0);
            write_u16(desc.add(10), 0);
            write_u8(desc.add(12), 0);
            write_u8(desc.add(13), 0);
            write_u16(desc.add(14), 0);
        }
        mmio_write32(mmio, REG_RDBAL, (rx_ring_phys & 0xFFFF_FFFF) as u32);
        mmio_write32(mmio, REG_RDBAH, (rx_ring_phys >> 32) as u32);
        mmio_write32(mmio, REG_RDLEN, (RX_RING_SIZE * 16) as u32);
        mmio_write32(mmio, REG_RDH, 0);
        mmio_write32(mmio, REG_RDT, (RX_RING_SIZE - 1) as u32);

        let rctl = RCTL_EN | RCTL_SBP | RCTL_UPE | RCTL_MPE | RCTL_BAM | RCTL_SECRC;
        mmio_write32(mmio, REG_RCTL, rctl);
    }

    // TX ring
    unsafe {
        for i in 0..TX_RING_SIZE {
            let desc = tx_ring_virt.add(i * 16);
            let buf_phys = tx_bufs_phys + (i * TX_BUF_SIZE) as u64;
            write_u64(desc, buf_phys);
            write_u16(desc.add(8), 0);
            write_u8(desc.add(10), 0);
            write_u8(desc.add(11), 0);
            write_u8(desc.add(12), TX_STATUS_DD);
            write_u8(desc.add(13), 0);
            write_u16(desc.add(14), 0);
        }
        mmio_write32(mmio, REG_TDBAL, (tx_ring_phys & 0xFFFF_FFFF) as u32);
        mmio_write32(mmio, REG_TDBAH, (tx_ring_phys >> 32) as u32);
        mmio_write32(mmio, REG_TDLEN, (TX_RING_SIZE * 16) as u32);
        mmio_write32(mmio, REG_TDH, 0);
        mmio_write32(mmio, REG_TDT, 0);

        let tctl = TCTL_EN | TCTL_PSP
            | (0x10 << TCTL_CT_SHIFT)
            | (0x40 << TCTL_COLD_SHIFT);
        mmio_write32(mmio, REG_TCTL, tctl);
    }

    let status = unsafe { mmio_read32(mmio, REG_STATUS) };
    let link_up = status & STATUS_LU != 0;

    crate::log_info!(
        "[e1000] MAC={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} \
         BAR0={:#010x} size={:#x} link={}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5],
        bar_phys, bar_size, if link_up { "up" } else { "down" }
    );

    *E1000.lock() = Some(E1000 {
        bar_phys, bar_size, mmio, mac,
        rx_ring_phys, rx_ring_virt,
        rx_bufs_phys, rx_bufs_virt,
        rx_cur: 0,
        tx_ring_phys, tx_ring_virt,
        tx_bufs_phys, tx_bufs_virt,
        tx_cur: 0,
    });
    true
}

pub fn mac() -> Option<[u8; 6]> {
    E1000.lock().as_ref().map(|d| d.mac)
}

pub fn send(frame: &[u8]) -> bool {
    if frame.len() < 14 || frame.len() > ETH_FRAME_MAX {
        crate::log_warn!("[e1000] send: bad len {}", frame.len());
        return false;
    }
    let mut guard = E1000.lock();
    let d = match guard.as_mut() {
        Some(x) => x,
        None => return false,
    };
    let idx = d.tx_cur;
    let desc = unsafe { d.tx_ring_virt.add(idx * 16) };
    let buf_phys = d.tx_bufs_phys + (idx * TX_BUF_SIZE) as u64;
    let buf_virt = unsafe { d.tx_bufs_virt.add(idx * TX_BUF_SIZE) };

    unsafe {
        core::ptr::copy_nonoverlapping(frame.as_ptr(), buf_virt, frame.len());
        write_u64(desc, buf_phys);
        write_u16(desc.add(8), frame.len() as u16);
        write_u8(desc.add(10), 0);
        write_u8(desc.add(11), TX_CMD_EOP | TX_CMD_IFCS | TX_CMD_RS);
        write_u8(desc.add(12), 0);
        write_u8(desc.add(13), 0);
        write_u16(desc.add(14), 0);

        let new_tail = ((idx + 1) % TX_RING_SIZE) as u32;
        mmio_write32(d.mmio, REG_TDT, new_tail);
        d.tx_cur = (idx + 1) % TX_RING_SIZE;
    }
    true
}

pub fn recv(out: &mut [u8]) -> Option<usize> {
    let mut guard = E1000.lock();
    let d = match guard.as_mut() {
        Some(x) => x,
        None => return None,
    };

    let idx = d.rx_cur;
    let desc = unsafe { d.rx_ring_virt.add(idx * 16) };
    let status = unsafe { read_u8(desc.add(12)) };
    if status & RX_STATUS_DD == 0 {
        return None;
    }

    let len = unsafe { read_u16(desc.add(8)) } as usize;
    if len == 0 || len > out.len() {
        unsafe { write_u8(desc.add(12), 0); }
        let new_tail = idx as u32;
        unsafe { mmio_write32(d.mmio, REG_RDT, new_tail); }
        d.rx_cur = (idx + 1) % RX_RING_SIZE;
        return None;
    }

    let buf = unsafe { d.rx_bufs_virt.add(idx * RX_BUF_SIZE) };
    unsafe { core::ptr::copy_nonoverlapping(buf, out.as_mut_ptr(), len); }

    unsafe { write_u8(desc.add(12), 0); }
    let new_tail = idx as u32;
    unsafe { mmio_write32(d.mmio, REG_RDT, new_tail); }
    d.rx_cur = (idx + 1) % RX_RING_SIZE;

    Some(len)
}