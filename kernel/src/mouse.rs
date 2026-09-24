use core::sync::atomic::{AtomicU64, Ordering};
use spin::Mutex;
use x86_64::instructions::interrupts::without_interrupts;
use x86_64::instructions::port::Port;

#[derive(Clone, Copy, Debug)]
pub enum MouseButton {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug)]
pub enum MouseEvent {
    ButtonDown(MouseButton),
    ButtonUp(MouseButton),
}

const EV_CAP: usize = 32;

struct EventQueue {
    buf: [Option<MouseEvent>; EV_CAP],
    head: usize,
    tail: usize,
    len: usize,
}

impl EventQueue {
    const fn new() -> Self {
        Self { buf: [None; EV_CAP], head: 0, tail: 0, len: 0 }
    }
    fn push(&mut self, ev: MouseEvent) {
        if self.len == EV_CAP { return; }
        self.buf[self.tail] = Some(ev);
        self.tail = (self.tail + 1) % EV_CAP;
        self.len += 1;
    }
    fn pop(&mut self) -> Option<MouseEvent> {
        if self.len == 0 { return None; }
        let ev = self.buf[self.head].take();
        self.head = (self.head + 1) % EV_CAP;
        self.len -= 1;
        ev
    }
}

static EVENTS: Mutex<EventQueue> = Mutex::new(EventQueue::new());
static POS: Mutex<(i32, i32)> = Mutex::new((640, 360));
static SCREEN: Mutex<(i32, i32)> = Mutex::new((1280, 720));

pub static BYTES_IRQ: AtomicU64 = AtomicU64::new(0);
pub static PACKETS_DONE: AtomicU64 = AtomicU64::new(0);
pub static IRQ_STARTS: AtomicU64 = AtomicU64::new(0);

pub fn set_screen(w: usize, h: usize) {
    let w = (w as i32).max(1);
    let h = (h as i32).max(1);
    *SCREEN.lock() = (w, h);
    let mut p = POS.lock();
    p.0 = p.0.clamp(0, w - 1);
    p.1 = p.1.clamp(0, h - 1);
}

pub fn screen_size() -> (i32, i32) {
    *SCREEN.lock()
}

pub fn position() -> (i32, i32) {
    without_interrupts(|| *POS.lock())
}

pub fn pop_event() -> Option<MouseEvent> {
    without_interrupts(|| EVENTS.lock().pop())
}

/// Вызывается из IRQ12 с очередным байтом от мыши.
pub fn push_byte_irq(byte: u8) {
    BYTES_IRQ.fetch_add(1, Ordering::Relaxed);
    handle_byte(byte);
}

fn handle_byte(byte: u8) {
    let (b, prev) = {
        let mut pkt = PACKET.lock();

        if pkt.idx == 0 && (byte & 0x08) == 0 {
            return;
        }

        let idx = pkt.idx;
        pkt.bytes[idx] = byte;
        pkt.idx += 1;

        if pkt.idx < 3 {
            return;
        }
        pkt.idx = 0;
        (pkt.bytes, pkt.prev_btn)
    };

    PACKETS_DONE.fetch_add(1, Ordering::Relaxed);

    if b[0] & 0xC0 != 0 {
        return;
    }

    let dx = if b[0] & 0x10 != 0 { b[1] as i32 - 256 } else { b[1] as i32 };
    let dy = if b[0] & 0x20 != 0 { b[2] as i32 - 256 } else { b[2] as i32 };
    let btn_bits = b[0] & 0x03;

    {
        let (sw, sh) = *SCREEN.lock();
        let mut p = POS.lock();
        p.0 = (p.0 + dx).clamp(0, sw - 1);
        p.1 = (p.1 - dy).clamp(0, sh - 1);
    }

    let mut q = EVENTS.lock();
    if (btn_bits & 0x01) != 0 && (prev & 0x01) == 0 {
        q.push(MouseEvent::ButtonDown(MouseButton::Left));
    }
    if (btn_bits & 0x01) == 0 && (prev & 0x01) != 0 {
        q.push(MouseEvent::ButtonUp(MouseButton::Left));
    }
    if (btn_bits & 0x02) != 0 && (prev & 0x02) == 0 {
        q.push(MouseEvent::ButtonDown(MouseButton::Right));
    }
    if (btn_bits & 0x02) == 0 && (prev & 0x02) != 0 {
        q.push(MouseEvent::ButtonUp(MouseButton::Right));
    }
    drop(q);

    PACKET.lock().prev_btn = btn_bits;
}

struct Packet {
    bytes: [u8; 3],
    idx: usize,
    prev_btn: u8,
}

static PACKET: Mutex<Packet> = Mutex::new(Packet { bytes: [0; 3], idx: 0, prev_btn: 0 });

// ---------- Инициализация i8042 ----------

unsafe fn wait_write() {
    let mut s: Port<u8> = Port::new(0x64);
    for _ in 0..100_000 {
        if s.read() & 0x02 == 0 { return; }
    }
    crate::serial_println!("[mouse] wait_write timeout");
}

unsafe fn wait_read() {
    let mut s: Port<u8> = Port::new(0x64);
    for _ in 0..100_000 {
        if s.read() & 0x01 != 0 { return; }
    }
    crate::serial_println!("[mouse] wait_read timeout");
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

    // 1. Включаем AUX port (мышь)
    wait_write();
    cmd.write(0xA8);

    // 2. Читаем текущий config byte
    wait_write();
    cmd.write(0x20);
    wait_read();
    let before = data.read();

    // 3. Устанавливаем bit 1 (IRQ12) и сбрасываем bit 5 (clock disable)
    let after = (before | 0x02) & !0x20;

    // 4. Пишем обратно
    wait_write();
    cmd.write(0x60);
    wait_write();
    data.write(after);

    // 5. Читаем обратно и проверяем
    wait_write();
    cmd.write(0x20);
    wait_read();
    let verify = data.read();

    crate::serial_println!(
        "[mouse] i8042 config: before={:#010b} wrote={:#010b} verify={:#010b}",
        before, after, verify
    );
    if verify & 0x02 == 0 {
        crate::serial_println!("[mouse] !!! IRQ12 не установлен в i8042 config !!!");
    }
    if verify & 0x20 != 0 {
        crate::serial_println!("[mouse] !!! clock мыши выключен (bit 5) !!!");
    }

    // 6. Set defaults
    mouse_write(0xF6);
    let a1 = mouse_read();
    crate::serial_println!("[mouse] F6 ack = {:#x} (ожидаем 0xFA)", a1);

    // 7. Enable data reporting
    mouse_write(0xF4);
    let a2 = mouse_read();
    crate::serial_println!("[mouse] F4 ack = {:#x} (ожидаем 0xFA)", a2);
}