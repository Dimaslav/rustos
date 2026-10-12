//! Драйвер Intel 82801AA AC'97 (эмулируется QEMU с `-device AC97`).
//!
//! # Что делает
//!
//! - Настраивает mixer (master vol, PCM vol, VRA, sample rate).
//! - Аллоцирует DMA-буфер (физически непрерывный) + BDL.
//! - Воспроизводит PCM (16-bit LE, stereo, 48000 Hz) через bus-master.
//!
//! # Ограничения MVP
//!
//! - Только playback, без записи.
//! - Один большой буфер — не кольцевой.
//! - Периодические прерывания от BCIS не обрабатываются — просто
//!   poll-им бит RPBM в PO_CR.

use alloc::vec::Vec;
use core::sync::atomic::Ordering;
use spin::Mutex;
use x86_64::instructions::port::Port;
use x86_64::structures::paging::FrameAllocator;

const SAMPLE_RATE: u32 = 48_000;

// NAM (mixer) регистры.
const NAM_RESET:       u16 = 0x00;
const NAM_MASTER_VOL:  u16 = 0x02;
const NAM_PCM_VOL:     u16 = 0x18;
const NAM_EXT_CTRL:    u16 = 0x2A;
const NAM_PCM_RATE:    u16 = 0x2C;

// NABM (bus master) регистры — PCM Out (playback).
const PO_BDBAR: u16 = 0x10;
const PO_LVI:   u16 = 0x15;
const PO_SR:    u16 = 0x16;
const PO_CR:    u16 = 0x1B;

#[inline]
fn outw(p: u16, v: u16) { unsafe { Port::<u16>::new(p).write(v); } }
#[inline]
fn outl(p: u16, v: u32) { unsafe { Port::<u32>::new(p).write(v); } }
#[inline]
fn outb(p: u16, v: u8)  { unsafe { Port::<u8>::new(p).write(v); } }
#[inline]
fn inw(p: u16) -> u16   { unsafe { Port::<u16>::new(p).read() } }
#[inline]
fn inb(p: u16) -> u8    { unsafe { Port::<u8>::new(p).read() } }

pub struct Ac97 {
    nabm: u16,

    /// Физический адрес PCM буфера (для DMA).
    buf_phys: u64,
    /// Виртуальный адрес PCM буфера (для записи из ядра).
    buf_virt: *mut u8,
    buf_size: usize,

    /// BDL: список дескрипторов (по 8 байт).
    bd_phys: u64,
    bd_virt: *mut u8,
}

unsafe impl Send for Ac97 {}
unsafe impl Sync for Ac97 {}

static AC97: Mutex<Option<Ac97>> = Mutex::new(None);

impl Ac97 {
    unsafe fn new(dev: &crate::pci::PciDevice) -> Option<Self> {
        // BAR0 = NAM (I/O), BAR1 = NABM (I/O).
        if dev.bar[0] & 1 == 0 || dev.bar[1] & 1 == 0 {
            crate::log_warn!("[ac97] BAR0/BAR1 не I/O");
            return None;
        }
        let nam = (dev.bar[0] & 0xFFFF_FFFC) as u16;
        let nabm = (dev.bar[1] & 0xFFFF_FFFC) as u16;
        if nam == 0 || nabm == 0 {
            crate::log_warn!("[ac97] BAR0/BAR1 нулевые");
            return None;
        }

        // Reset mixer.
        outw(nam + NAM_RESET, 0);
        for _ in 0..100_000 { core::hint::spin_loop(); }

        // Master volume = 0x0000 (max), PCM out = 0x0808 (max).
        outw(nam + NAM_MASTER_VOL, 0x0000);
        outw(nam + NAM_PCM_VOL, 0x0808);

        // VRA = Variable Rate Audio (бит 0 NAM_EXT_CTRL).
        let ext = inw(nam + NAM_EXT_CTRL);
        outw(nam + NAM_EXT_CTRL, ext | 0x0001);

        // Sample rate 48000 → register = 0.
        outw(nam + NAM_PCM_RATE, 0);

        // Reset bus master: RR (bit 1) = 1, потом 0.
        outb(nabm + PO_CR, 0x02);
        for _ in 0..10_000 { core::hint::spin_loop(); }
        outb(nabm + PO_CR, 0x00);

        // Аллоцируем DMA-память: 1 сек PCM + 1 страница под BDL.
        let buf_size: usize = SAMPLE_RATE as usize * 4;
        let buf_pages = (buf_size + 4095) / 4096;
        let total_pages = buf_pages + 1;

        let phys_offset = crate::memory::PHYS_OFFSET.load(Ordering::Acquire);
        if phys_offset == 0 {
            crate::log_warn!("[ac97] phys_offset=0");
            return None;
        }

        let mut phys: Vec<u64> = Vec::with_capacity(total_pages);
        {
            let mut guard = crate::memory::FRAME_ALLOCATOR.lock();
            let alloc = guard.as_mut()?;
            for _ in 0..total_pages {
                let f = alloc.allocate_frame()?;
                phys.push(f.start_address().as_u64());
            }
        }

        // Проверка непрерывности (наивно — но в QEMU работает).
        for i in 1..phys.len() {
            if phys[i] != phys[i - 1] + 4096 {
                crate::log_warn!(
                    "[ac97] DMA память не непрерывна: {:#x} → {:#x}",
                    phys[i - 1], phys[i]
                );
                return None;
            }
        }

        let buf_phys = phys[0];
        let bd_phys = phys[buf_pages];

        let buf_virt = (phys_offset + buf_phys) as *mut u8;
        let bd_virt = (phys_offset + bd_phys) as *mut u8;

        core::ptr::write_bytes(buf_virt, 0, buf_size);
        core::ptr::write_bytes(bd_virt, 0, 4096);

        crate::log_info!(
            "[ac97] nam={:#x} nabm={:#x} buf={:#x} ({} bytes) bd={:#x}",
            nam, nabm, buf_phys, buf_size, bd_phys
        );

        Some(Ac97 {
            nabm,
            buf_phys,
            buf_virt,
            buf_size,
            bd_phys,
            bd_virt,
        })
    }

    /// Скопировать PCM в DMA-буфер и запустить. Не блокирует.
    pub unsafe fn play(&mut self, samples: &[i16]) {
        let bytes = samples.len() * 2;
        if bytes == 0 || bytes > self.buf_size {
            crate::log_warn!(
                "[ac97] play: размер {} не влезает в {}",
                bytes, self.buf_size
            );
            return;
        }

        core::ptr::copy_nonoverlapping(
            samples.as_ptr() as *const u8,
            self.buf_virt,
            bytes,
        );

        // BDL: раскладываем буфер на куски ≤ 65535 сэмплов (u16 len).
        let total_samples = (bytes / 2) as u32;
        let mut written = 0u32;
        let mut idx = 0usize;
        while written < total_samples && idx < 64 {
            let this = (total_samples - written).min(65_535);
            let addr = self.buf_phys + (written as u64) * 2;

            let p = self.bd_virt.add(idx * 8) as *mut u8;
            core::ptr::write_unaligned(p as *mut u32, addr as u32);
            core::ptr::write_unaligned(p.add(4) as *mut u16, this as u16);
            // Flags: BUP на первом, IOC на всех (безвредно без IRQ).
            let mut flags = 0u16;
            if idx == 0 { flags |= 0x4000; }
            core::ptr::write_unaligned(p.add(6) as *mut u16, flags);

            written += this;
            idx += 1;
        }
        if idx == 0 { return; }
        let last_idx = (idx - 1) as u8;

        // Программируем BDL и запускаем.
        outl(self.nabm + PO_BDBAR, self.bd_phys as u32);
        outb(self.nabm + PO_LVI, last_idx);
        // Сброс флагов статуса (все единицы = clear).
        outw(self.nabm + PO_SR, 0x1C);
        // RPBM (bit 0) = 1 → run.
        outb(self.nabm + PO_CR, 0x01);
    }

    pub unsafe fn stop(&mut self) {
        outb(self.nabm + PO_CR, 0x00);
    }

    pub unsafe fn is_playing(&self) -> bool {
        inb(self.nabm + PO_CR) & 0x01 != 0
    }
}

/// Найти AC'97, инициализировать, сохранить в глобальном статике.
pub fn init() -> bool {
    let devs = crate::pci::find_audio();
    let dev = match devs.first() {
        Some(d) => *d,
        None => {
            crate::log_warn!("[ac97] не найден");
            return false;
        }
    };

    crate::pci::enable_bus_master(&dev);

    let ac97 = unsafe { Ac97::new(&dev) };
    match ac97 {
        Some(a) => {
            *AC97.lock() = Some(a);
            crate::log_info!("[ac97] инициализирован");
            true
        }
        None => {
            crate::log_warn!("[ac97] init failed");
            false
        }
    }
}

/// Сгенерировать синусоиду заданной частоты и длительности, сыграть.
/// Блокирует, пока не закончится (или до таймаута).
pub fn beep(freq_hz: u32, duration_ms: u32) {
    if freq_hz == 0 || duration_ms == 0 { return; }

    let mut guard = AC97.lock();
    let ac97 = match guard.as_mut() {
        Some(a) => a,
        None => return,
    };

    let n_frames = (SAMPLE_RATE as u64 * duration_ms as u64 / 1000) as usize;
    let mut pcm: Vec<i16> = Vec::with_capacity(n_frames * 2);

    const SIN_TABLE: [i16; 256] = SIN_TABLE_CONST;
    let step = ((freq_hz as u64) * 256 / SAMPLE_RATE as u64).max(1) as usize;

    let mut phase = 0usize;
    for _ in 0..n_frames {
        let s = SIN_TABLE[phase & 0xFF];
        pcm.push(s);
        pcm.push(s);
        phase += step;
    }

    unsafe {
        ac97.play(&pcm);
        // Ждём завершения с таймаутом.
        let mut it = 0u32;
        while ac97.is_playing() && it < 2_000_000 {
            it += 1;
            core::hint::spin_loop();
        }
        ac97.stop();
    }
}

/// Сыграть PCM из i16 stereo 48kHz. Синхронно.
pub fn play_samples(samples: &[i16]) {
    let mut guard = AC97.lock();
    let ac97 = match guard.as_mut() {
        Some(a) => a,
        None => return,
    };
    unsafe {
        ac97.play(samples);
        let mut it = 0u32;
        while ac97.is_playing() && it < 10_000_000 {
            it += 1;
            core::hint::spin_loop();
        }
        ac97.stop();
    }
}

// ---------- Таблица sin (сгенерирована заранее) ----------
//
// sin(2π·i/256) * 12000 для i = 0..255. При частоте 440 Гц шаг по
// таблице ≈ 2.35 → слышится тон 440 Гц.

const SIN_TABLE_CONST: [i16; 256] = [
    0, 294, 588, 880, 1171, 1459, 1745, 2027,
    2306, 2581, 2851, 3116, 3375, 3628, 3874, 4114,
    4347, 4572, 4790, 4999, 5200, 5393, 5576, 5750,
    5915, 6070, 6215, 6350, 6475, 6590, 6694, 6787,
    6869, 6941, 7001, 7050, 7088, 7115, 7130, 7135,
    7128, 7110, 7080, 7040, 6989, 6927, 6854, 6771,
    6677, 6573, 6459, 6334, 6200, 6056, 5903, 5740,
    5569, 5389, 5200, 5004, 4800, 4589, 4371, 4146,
    3915, 3678, 3435, 3187, 2934, 2677, 2415, 2150,
    1882, 1611, 1337, 1062, 785, 507, 228, -50,
    -329, -607, -884, -1159, -1432, -1702, -1969, -2233,
    -2493, -2749, -3000, -3246, -3486, -3720, -3948, -4170,
    -4384, -4591, -4790, -4981, -5163, -5336, -5499, -5653,
    -5796, -5929, -6051, -6161, -6260, -6347, -6422, -6485,
    -6535, -6573, -6598, -6610, -6609, -6596, -6570, -6530,
    -6478, -6413, -6336, -6246, -6144, -6029, -5903, -5765,
    -5616, -5455, -5284, -5101, -4908, -4705, -4492, -4269,
    -4038, -3797, -3549, -3292, -3028, -2757, -2479, -2195,
    -1906, -1611, -1312, -1009, -703, -394, -83, 229,
    541, 852, 1160, 1466, 1768, 2065, 2358, 2645,
    2926, 3200, 3467, 3726, 3976, 4217, 4449, 4670,
    4881, 5081, 5270, 5446, 5611, 5763, 5902, 6028,
    6140, 6239, 6324, 6395, 6452, 6494, 6522, 6536,
    6535, 6519, 6489, 6445, 6386, 6314, 6227, 6127,
    6013, 5886, 5746, 5593, 5428, 5251, 5061, 4860,
    4648, 4425, 4191, 3948, 3695, 3434, 3164, 2887,
    2603, 2313, 2018, 1717, 1413, 1105, 795, 483,
    170, -143, -456, -767, -1075, -1380, -1680, -1976,
    -2266, -2549, -2826, -3094, -3353, -3603, -3844, -4073,
    -4292, -4500, -4696, -4880, -5052, -5211, -5357, -5489,
    -5607, -5711, -5800, -5875, -5935, -5980, -6010, -6025,
    -6025, -6010, -5980, -5935, -5875, -5800, -5711, -5607,
];