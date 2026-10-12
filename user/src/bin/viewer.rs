#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::vec::Vec;
use userlib::*;

const WIN_W: u64 = 640;
const WIN_H: u64 = 480;
const MAX_FILE: usize = 16 * 1024 * 1024;

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

fn fill_win(buf: *mut u8, c: (u8, u8, u8)) {
    for i in 0..(WIN_W * WIN_H) {
        let off = (i * 4) as usize;
        unsafe {
            *buf.add(off)     = c.2;
            *buf.add(off + 1) = c.1;
            *buf.add(off + 2) = c.0;
            *buf.add(off + 3) = 255;
        }
    }
}

/// Nearest-neighbour scale RGB → BGRA.
fn draw_scaled(buf: *mut u8, src: *const u8, sw: usize, sh: usize) {
    if sw == 0 || sh == 0 { return; }
    let w = WIN_W as usize;
    let h = WIN_H as usize;
    for dy in 0..h {
        let sy = dy * sh / h;
        for dx in 0..w {
            let sx = dx * sw / w;
            let s = (sy * sw + sx) * 3;
            let d = (dy * w + dx) * 4;
            let (r, g, b) = unsafe { (*src.add(s), *src.add(s + 1), *src.add(s + 2)) };
            unsafe {
                *buf.add(d)     = b;
                *buf.add(d + 1) = g;
                *buf.add(d + 2) = r;
                *buf.add(d + 3) = 255;
            }
        }
    }
}

/// Процедурный градиент с кружком — для проверки viewer без файлов.
fn procedural_rgb(w: usize, h: usize) -> Vec<u8> {
    let mut v: Vec<u8> = alloc::vec![0u8; w * h * 3];
    let cx = w as i32 / 2;
    let cy = h as i32 / 2;
    let r2 = (w.min(h) as i32 / 3).pow(2);
    for y in 0..h {
        for x in 0..w {
            let off = (y * w + x) * 3;
            let dx = x as i32 - cx;
            let dy = y as i32 - cy;
            let d2 = dx * dx + dy * dy;
            let (r, g, b) = if d2 < r2 {
                (255u8, 240, 90)
            } else {
                let rr = (x * 255 / w.max(1)) as u8;
                let gg = (y * 255 / h.max(1)) as u8;
                let bb = 180u8;
                (rr, gg, bb)
            };
            v[off] = r;
            v[off + 1] = g;
            v[off + 2] = b;
        }
    }
    v
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    let args = unsafe { collect_args(argc, argv) };

    let (w, h, rgb_ptr, rgb_size, label);
    let keep_buf: Option<Vec<u8>>;

    if args.len() >= 2 {
        let path = &args[1];
        let fd = open(path);
        if fd == u64::MAX {
            write(b"viewer: cannot open: ");
            write(path.as_bytes());
            write(b"\n");
            exit(1);
        }
        let data = read_all(fd);
        close(fd);

        let dims = match image_info(&data) {
            Some(d) => d,
            None => {
                write(b"viewer: decode failed (not PNG/BMP?)\n");
                exit(1);
            }
        };
        let (iw, ih) = dims;

        let bytes = iw * ih * 3;
        let ptr = mmap(bytes);
        if ptr == u64::MAX {
            write(b"viewer: mmap failed\n");
            exit(1);
        }
        if image_decode(&data, ptr, bytes).is_none() {
            write(b"viewer: decode2 failed\n");
            exit(1);
        }

        // Держим `data` живым до конца — но она уже скопирована в ptr.
        // `data` можно освободить (bump allocator не освобождает, но
        // это не критично для MVP).
        drop(data);

        w = iw;
        h = ih;
        rgb_ptr = ptr;
        rgb_size = bytes;
        label = path.clone();
        keep_buf = None;
    } else {
        // Без аргументов — процедурный градиент.
        let iw = 320usize;
        let ih = 240usize;
        let v = procedural_rgb(iw, ih);
        let bytes = v.len();
        let ptr = mmap(bytes);
        if ptr == u64::MAX {
            write(b"viewer: mmap failed\n");
            exit(1);
        }
        unsafe {
            core::ptr::copy_nonoverlapping(v.as_ptr(), ptr as *mut u8, bytes);
        }
        w = iw;
        h = ih;
        rgb_ptr = ptr;
        rgb_size = bytes;
        label = alloc::string::String::from("(procedural)");
        keep_buf = Some(v);
    }

    let msg = format!("viewer: {} ({}x{})\n", label, w, h);
    write(msg.as_bytes());

    let win_id = create_window(WIN_W, WIN_H, b"Viewer\0");
    if win_id == u64::MAX {
        write(b"viewer: create_window failed\n");
        exit(1);
    }

    let buf = WIN_BUF_VADDR as *mut u8;
    fill_win(buf, (20, 20, 20));
    draw_scaled(buf, rgb_ptr as *const u8, w, h);
    present(win_id);

    set_focus(true);

    loop {
        // Клавиатура — Esc закрывает.
        let k = poll_key();
        if k == 0x1B {
            set_focus(false);
            destroy_window(win_id);
            exit(0);
        }

        // События окна.
        if let Some(ev) = poll_event(win_id) {
            let _ = ev;
        } else {
            sleep_ms(30);
        }

        let _ = rgb_size;
        let _ = &keep_buf;
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { loop {} }