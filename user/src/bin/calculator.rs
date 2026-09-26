#![no_std]
#![no_main]

use userlib::*;

const W: usize = 200;
const H: usize = 240;

const BG: (u8, u8, u8) = (40, 44, 52);
const BTN: (u8, u8, u8) = (60, 66, 78);
const BTN_HOVER: (u8, u8, u8) = (90, 100, 130);
const TEXT_BG: (u8, u8, u8) = (24, 26, 32);
const FG: (u8, u8, u8) = (235, 240, 250);

struct Calc {
    buf: *mut u8,
    display: u64,
    a: u64,
    op: char,
    fresh: bool,
}

impl Calc {
    fn clear(&mut self) {
        for y in 0..H {
            for x in 0..W {
                let c = if y < 44 { TEXT_BG } else { BG };
                put_pixel(self.buf, W, x, y, c.0, c.1, c.2);
            }
        }
    }
    fn fill(&mut self, x0: usize, y0: usize, w: usize, h: usize, c: (u8, u8, u8)) {
        for y in y0..y0 + h {
            for x in x0..x0 + w {
                put_pixel(self.buf, W, x, y, c.0, c.1, c.2);
            }
        }
    }
    fn digit(&mut self, x: usize, y: usize, d: u8, c: (u8, u8, u8)) {
        const DIG: [[u8; 5]; 10] = [
            [0b01110,0b10001,0b10011,0b10101,0b11001],
            [0b00100,0b01100,0b00100,0b00100,0b01110],
            [0b01110,0b10001,0b00110,0b01000,0b11111],
            [0b11111,0b00010,0b00100,0b00010,0b11111],
            [0b00010,0b00110,0b01010,0b11111,0b00010],
            [0b11111,0b10000,0b11110,0b00001,0b11110],
            [0b00110,0b01000,0b11110,0b10001,0b01110],
            [0b11111,0b00001,0b00010,0b00100,0b00100],
            [0b01110,0b10001,0b01110,0b10001,0b01110],
            [0b01110,0b10001,0b01111,0b00010,0b01100],
        ];
        let rows = DIG[(d as usize).min(9)];
        for (ry, row) in rows.iter().enumerate() {
            for rx in 0..5 {
                if (row >> (4 - rx)) & 1 == 1 {
                    self.fill(x + rx * 2, y + ry * 2, 2, 2, c);
                }
            }
        }
    }
    fn number(&mut self, x0: usize, y: usize, mut n: u64) {
        let mut digits = [0u8; 20];
        let mut count = 0;
        if n == 0 { digits[0] = 0; count = 1; }
        else { while n > 0 { digits[count] = (n % 10) as u8; n /= 10; count += 1; } }
        let mut x = x0;
        for i in (0..count).rev() {
            self.digit(x, y, digits[i], FG);
            x += 12;
        }
    }
    fn button(&mut self, col: usize, row: usize, hover: bool) {
        let bx = 6 + col * 48;
        let by = 50 + row * 48;
        let c = if hover { BTN_HOVER } else { BTN };
        self.fill(bx, by, 44, 44, c);
    }
    fn draw_all(&mut self, hc: Option<usize>, hr: Option<usize>) {
        self.clear();
        self.number(10, 14, self.display);
        for r in 0..4 {
            for c in 0..4 {
                self.button(c, r, hc == Some(c) && hr == Some(r));
            }
        }
    }
    fn hit(&self, x: i32, y: i32) -> Option<(usize, usize)> {
        let bx = x - 6;
        let by = y - 50;
        if bx < 0 || by < 0 { return None; }
        let col = (bx / 48) as usize;
        let row = (by / 48) as usize;
        if col >= 4 || row >= 4 { return None; }
        Some((col, row))
    }
    fn click(&mut self, col: usize, row: usize) {
        let ch: u8 = match (row, col) {
            (0, 0) => b'7', (0, 1) => b'8', (0, 2) => b'9', (0, 3) => b'/',
            (1, 0) => b'4', (1, 1) => b'5', (1, 2) => b'6', (1, 3) => b'*',
            (2, 0) => b'1', (2, 1) => b'2', (2, 2) => b'3', (2, 3) => b'-',
            (3, 0) => b'0', (3, 1) => b'.', (3, 2) => b'=', (3, 3) => b'+',
            _ => return,
        };
        if ch.is_ascii_digit() {
            if self.fresh { self.display = 0; self.fresh = false; }
            self.display = self.display * 10 + (ch - b'0') as u64;
        } else if ch == b'=' {
            let b = self.display;
            let r = match self.op {
                '+' => self.a + b,
                '-' => self.a.saturating_sub(b),
                '*' => self.a * b,
                '/' => if b != 0 { self.a / b } else { 0 },
                _ => b,
            };
            self.display = r;
            self.a = 0;
            self.op = ' ';
            self.fresh = true;
        } else {
            self.a = self.display;
            self.op = ch as char;
            self.fresh = true;
        }
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    write(b"[calc] starting\n");
    let win_id = create_window(W as u64, H as u64, b"Calculator\0");
    if win_id == u64::MAX {
        write(b"[calc] create_window failed\n");
        exit(1);
    }
    write(b"[calc] window created\n");

    let mut calc = Calc {
        buf: WIN_BUF_VADDR as *mut u8,
        display: 0,
        a: 0,
        op: ' ',
        fresh: true,
    };
    calc.draw_all(None, None);
    present(win_id);

    loop {
        if let Some(ev) = poll_event(win_id) {
            if ev.kind == 0 {
                if let Some((col, row)) = calc.hit(ev.x, ev.y) {
                    calc.click(col, row);
                    calc.draw_all(Some(col), Some(row));
                    present(win_id);
                }
            }
        } else {
            sleep_ms(30);
        }
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { loop {} }