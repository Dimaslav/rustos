//! Smoke-тесты. Запускаются в `kernel_main` при `feature = "headless_test"`.
//!
//! Формат вывода машиночитаемый:
//!   `[TEST PASS] <name>`
//!   `[TEST FAIL] <name>: <reason>`
//!   `[TEST SKIP] <name> (<reason>)`
//! и в конце: `SMOKE_TEST_PASS` или `SMOKE_TEST_FAIL`.
//!
//! `src/main.rs` (root) парсит именно последние строки.

use crate::serial_println;

struct Stats {
    passed: u32,
    failed: u32,
}

impl Stats {
    fn new() -> Self { Self { passed: 0, failed: 0 } }

    fn pass(&mut self, name: &str) {
        serial_println!("[TEST PASS] {}", name);
        self.passed += 1;
    }
    fn fail(&mut self, name: &str, reason: &str) {
        serial_println!("[TEST FAIL] {}: {}", name, reason);
        self.failed += 1;
    }
    fn skip(&mut self, name: &str, reason: &str) {
        serial_println!("[TEST SKIP] {} ({})", name, reason);
    }
    fn check(&mut self, name: &str, ok: bool, reason: &str) {
        if ok { self.pass(name) } else { self.fail(name, reason) }
    }
}

pub fn run_all() -> u32 {
    serial_println!("[TEST] === smoke tests begin ===");
    let mut s = Stats::new();

    // ---------- 1. heap ----------
    {
        use alloc::vec::Vec;
        let v: Vec<u8> = (0..1000).map(|i| (i & 0xFF) as u8).collect();
        s.check("heap_alloc_1000",
            v.len() == 1000 && v[500] == ((500u32 & 0xFF) as u8),
            "Vec len or content mismatch");
    }

    // ---------- 2. format / String / Vec ----------
    {
        use alloc::format;
        use alloc::string::String;
        use alloc::vec::Vec;

        let str_val: String = format!("{}-{}", 1, 2);
        s.check("format_basic", str_val == "1-2", "format! вернул не то");

        let mut v: Vec<u32> = Vec::new();
        for i in 0..100 { v.push(i); }
        let sum: u32 = v.iter().sum();
        s.check("vec_sum_4950", sum == 4950, "сумма != 4950");
    }

    // ---------- 3. RAMFS ----------
    {
        let created = crate::vfs::ramfs_create_file("/", "smoke.txt", b"hello");
        s.check("ramfs_create_file", created, "create_file вернул false");

        let data = crate::vfs::ramfs_read("/smoke.txt");
        s.check("ramfs_read_back",
            data.as_deref() == Some(b"hello"),
            "содержимое не совпало");

        let mk = crate::vfs::ramfs_mkdir("/", "smokedir");
        s.check("ramfs_mkdir", mk, "mkdir вернул false");

        let exists = crate::vfs::ramfs_exists("/smokedir");
        s.check("ramfs_exists_after_mkdir", exists, "mkdir не создал директорию");

        let rm = crate::vfs::ramfs_remove("/smoke.txt");
        s.check("ramfs_remove", rm, "remove вернул false");
    }

    // ---------- 4. VFS API ----------
    {
        let list = crate::vfs::list("/");
        let has = list.iter().any(|e| e.name == "smokedir");
        s.check("vfs_list_root", has, "smokedir не видно в листинге /");
    }

    // ---------- 5. FAT32 (soft — диск может отсутствовать) ----------
    {
        let entries = crate::vfs::fat32_list_root();
        if entries.is_empty() {
            s.skip("fat32_mounted", "no FAT32 mounted");
        } else {
            let fake = crate::vfs::fat32_exists("__no_such_file__.txt");
            s.check("fat32_exists_false_for_missing", !fake, "нашёлся несуществующий файл");
            serial_println!("[TEST INFO] fat32 entries = {}", entries.len());
            s.pass("fat32_mounted");
        }
    }

    // ---------- 6. Interrupts / ticks ----------
    {
        let t0 = crate::interrupts::ticks();
        crate::sched::sleep_ms(100);
        let t1 = crate::interrupts::ticks();
        s.check("timer_advances", t1 > t0,
            "ticks не растут (IRQ0 не работает?)");
    }

    // ---------- 7. Scheduler ----------
    {
        use core::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);

        extern "C" fn test_thread() -> ! {
            COUNTER.fetch_add(1, Ordering::SeqCst);
            crate::sched::exit_current();
        }

        crate::sched::spawn("smoke", test_thread);
        crate::sched::sleep_ms(200);
        let n = COUNTER.load(Ordering::SeqCst);
        s.check("sched_spawn_run", n >= 1, "test_thread не выполнился");
    }

    serial_println!("[TEST] === smoke tests end: passed={} failed={} ===",
        s.passed, s.failed);
    s.failed
}