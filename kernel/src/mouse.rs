use spin::Mutex;
use x86_64::instructions::port::Port;

#[derive(Clone, Copy, Debug, Default)]
pub struct MouseEvent {
    pub x: i32,
    pub y: i32,
    pub left: bool,
    pub right: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct MouseState {
    pub x: i32,
    pub y: i32,
    pub left: bool,
    pub right: bool,
    pub dirty: bool,
    // Для debounce
    pub last_click_tick: u64,
    pub click_pending: bool,
}

pub static MOUSE: Mutex<MouseState> = Mutex::new(MouseState {
    x: 640,
    y: 360,
    left: false,
    right: false,
    dirty: true,
    last_click_tick: 0,
    click_pending: false,
});

struct Packet {
    bytes: [u8; 3],
    idx: usize,
}

static PACKET: Mutex<Packet> = Mutex::new(Packet {
    bytes: [0; 3],
    idx: 0,
});

unsafe fn wait_write() {
    let mut s: Port<u8> = Port::new(0x64);
    for _ in 0..100_000 {
        if s.read() & 0x02 == 0 { return; }
    }
}

unsafe fn wait_read() {
    let mut s: Port<u8> = Port::new(0x64);
    for _ in 0..100_000 {
        if s.read() & 0x01 != 0 { return; }
    }
}

unsafe fn mouse_write(v: u8) {
    let mut cmd: Port<u8> = Port::new(0x64);
    let mut data: Port<u8> = Port::new(0x60);
    wait_write();
    cmd.write(0xD4);
    wait_write();
    data.write(v);
}

unsafe fn mouse_read() -> u8 {
    let mut data: Port<u8> = Port::new(0x60);
    wait_read();
    data.read()
}

pub unsafe fn init() {
    let mut cmd: Port<u8> = Port::new(0x64);
    let mut data: Port<u8> = Port::new(0x60);

    wait_write();
    cmd.write(0xA8);

    wait_write();
    cmd.write(0x20);
    wait_read();
    let mut config = data.read();

    config |= 0x02;    // включить IRQ12
    config &= !0x20;   // включить тактирование мыши

    wait_write();
    cmd.write(0x60);
    wait_write();
    data.write(config);

    mouse_write(0xF6);
    let _ = mouse_read();
    mouse_write(0xF4);
    let _ = mouse_read();
}

pub fn push_byte(byte: u8) {
    let mut pkt = PACKET.lock();
    let idx = pkt.idx;
    pkt.bytes[idx] = byte;
    pkt.idx += 1;

    if pkt.idx < 3 { return; }
    pkt.idx = 0;
    let b = pkt.bytes;

    if b[0] & 0x08 == 0 { return; }

    let dx = b[1] as i32 - if b[0] & 0x10 != 0 { 256 } else { 0 };
    let dy = b[2] as i32 - if b[0] & 0x20 != 0 { 256 } else { 0 };
    let left = b[0] & 0x01 != 0;
    let right = b[0] & 0x02 != 0;

    let mut s = MOUSE.lock();

    // Применяем смещение с фильтром: игнорируем мелкий шум
    if dx.abs() > 0 || dy.abs() > 0 {
        s.x = (s.x + dx).clamp(0, 1279);
        s.y = (s.y - dy).clamp(0, 719);
        s.dirty = true;
    }

    // Debounce нажатия: реагируем только на переход 0→1,
    // но не чаще, чем раз в 5 тиков таймера (~275 мс).
    if left && !s.left {
        let now = crate::interrupts::ticks();
        if now.saturating_sub(s.last_click_tick) >= 5 || s.last_click_tick == 0 {
            s.last_click_tick = now;
            s.click_pending = true;
        }
        s.left = true;
        s.dirty = true;
    } else if !left && s.left {
        s.left = false;
        s.dirty = true;
    }
    s.right = right;
}

/// Забрать накопившийся «щелчок» (edge), если он есть.
pub fn take_click() -> Option<(i32, i32, bool, bool)> {
    let mut s = MOUSE.lock();
    if s.click_pending {
        s.click_pending = false;
        Some((s.x, s.y, s.left, s.right))
    } else {
        None
    }
}