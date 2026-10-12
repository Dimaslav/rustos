#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::vec::Vec;
use userlib::*;

const MAX_FILE: usize = 32 * 1024 * 1024;
/// Фреймов в одном куске PCM. ~0.46 сек stereo при 48000 Hz.
/// Держим ≤ 96000 i16 samples (≈192 KB), чтобы влезло в DMA-буфер AC'97.
const CHUNK_FRAMES: usize = 22050;

fn read_all(fd: u64) -> Vec<u8> {
    let mut buf: Vec<u8> = Vec::new();
    let mut tmp = [0u8; 4096];
    loop {
        let n = read(fd, tmp.as_mut_ptr() as u64, tmp.len() as u64);
        if n == 0 || n == u64::MAX { break; }
        buf.extend_from_slice(&tmp[..n as usize]);
        if buf.len() > MAX_FILE { break; }
    }
    buf
}

fn read_u16_le(b: &[u8], off: usize) -> u16 {
    (b[off] as u16) | ((b[off + 1] as u16) << 8)
}

fn read_u32_le(b: &[u8], off: usize) -> u32 {
    (b[off] as u32)
        | ((b[off + 1] as u32) << 8)
        | ((b[off + 2] as u32) << 16)
        | ((b[off + 3] as u32) << 24)
}

struct WavInfo {
    channels: u16,
    sample_rate: u32,
    bits: u16,
    data_off: usize,
    data_len: usize,
}

fn parse_wav(b: &[u8]) -> Option<WavInfo> {
    if b.len() < 44 { return None; }
    if &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" { return None; }

    let mut off = 12usize;
    let mut fmt_ok = false;
    let mut channels = 0u16;
    let mut sample_rate = 0u32;
    let mut bits = 0u16;
    let mut data_off = 0usize;
    let mut data_len = 0usize;

    while off + 8 <= b.len() {
        let id = &b[off..off + 4];
        let sz = read_u32_le(b, off + 4) as usize;
        let body_off = off + 8;
        if body_off + sz > b.len() { break; }

        if id == b"fmt " {
            if sz < 16 { return None; }
            let audio_format = read_u16_le(b, body_off);
            if audio_format != 1 { return None; }
            channels = read_u16_le(b, body_off + 2);
            sample_rate = read_u32_le(b, body_off + 4);
            bits = read_u16_le(b, body_off + 14);
            fmt_ok = true;
        } else if id == b"data" {
            data_off = body_off;
            data_len = sz;
        }

        off = body_off + sz;
        if off & 1 != 0 { off += 1; }
    }

    if !fmt_ok || data_off == 0 || data_len == 0 { return None; }
    Some(WavInfo { channels, sample_rate, bits, data_off, data_len })
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    let args = unsafe { collect_args(argc, argv) };
    if args.len() < 2 {
        write(b"usage: play FILE.wav\n");
        exit(1);
    }

    let path = &args[1];
    let fd = open(path);
    if fd == u64::MAX {
        write(b"play: cannot open: ");
        write(path.as_bytes());
        write(b"\n");
        exit(1);
    }
    let data = read_all(fd);
    close(fd);

    let info = match parse_wav(&data) {
        Some(i) => i,
        None => {
            write(b"play: not a valid PCM WAV file\n");
            exit(1);
        }
    };

    if info.bits != 16 {
        write(b"play: only 16-bit PCM supported\n");
        exit(1);
    }
    if info.channels != 1 && info.channels != 2 {
        write(b"play: only mono/stereo supported\n");
        exit(1);
    }

    let msg = format!(
        "play: {} Hz, {} ch, {} bits, {} bytes\n",
        info.sample_rate, info.channels, info.bits, info.data_len
    );
    write(msg.as_bytes());

    let pcm_bytes = &data[info.data_off..info.data_off + info.data_len];
    let total_samples = info.data_len / 2;
    let src_frames = total_samples / info.channels as usize;
    if src_frames == 0 {
        write(b"play: empty data\n");
        exit(0);
    }

    // Nearest-neighbour ресемплинг до 48000 Hz + interleaved stereo.
    let src_rate = info.sample_rate.max(1);
    let dst_rate: u32 = 48000;
    let out_frames = ((src_frames as u64) * (dst_rate as u64) / (src_rate as u64)) as usize;

    let mut out: Vec<i16> = Vec::with_capacity(out_frames * 2);
    for i in 0..out_frames {
        let src_frame = ((i as u64) * (src_rate as u64) / (dst_rate as u64)) as usize;
        let src_frame = src_frame.min(src_frames.saturating_sub(1));
        let base = src_frame * info.channels as usize;

        let l = if base < total_samples {
            i16::from_le_bytes([pcm_bytes[base * 2], pcm_bytes[base * 2 + 1]])
        } else {
            0
        };
        let r = if info.channels == 2 && base + 1 < total_samples {
            i16::from_le_bytes([pcm_bytes[(base + 1) * 2], pcm_bytes[(base + 1) * 2 + 1]])
        } else {
            l
        };
        out.push(l);
        out.push(r);
    }

    // Играем кусками.
    let chunk_samples = CHUNK_FRAMES * 2;
    let total_chunks = (out.len() + chunk_samples - 1) / chunk_samples;
    for (ci, chunk) in out.chunks(chunk_samples).enumerate() {
        if ci % 10 == 0 || ci + 1 == total_chunks {
            let done = (ci * 100) / total_chunks.max(1);
            let m = format!("play: {}%\n", done);
            write(m.as_bytes());
        }
        sound_play_pcm(chunk);
    }

    write(b"play: done\n");
    exit(0);
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { loop {} }