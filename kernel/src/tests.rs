//! Smoke-тесты. Запускаются в `kernel_main` при `feature = "headless_test"`.

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
        let list = crate::vfs::list("/").ok().unwrap_or_default();
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

    // ---------- 7. Scheduler: spawn + exit code + wait ----------
    {
        extern "C" fn test_thread() -> ! {
            crate::sched::exit_current_with_code(42);
        }

        let id = crate::sched::spawn("smoke_child", test_thread);
        let code = crate::sched::wait_for(id);
        s.check("wait_exit_code_42", code == Some(42), "exit code не 42");
    }

    // ---------- 8. Scheduler: kill ----------
    {
        extern "C" fn slow_thread() -> ! {
            loop { crate::sched::sleep_ms(1000); }
        }

        let id = crate::sched::spawn("smoke_kill", slow_thread);
        crate::sched::sleep_ms(50);
        let killed = crate::sched::kill(id);
        s.check("kill_returns_true", killed, "kill вернул false");

        let code = crate::sched::wait_for(id);
        s.check("kill_exit_code_neg", code == Some(-9), "ожидали exit_code = -9");
    }

    // ---------- 9. getppid ----------
    {
        use core::sync::atomic::{AtomicU64, Ordering};
        static CHILD_PPID: AtomicU64 = AtomicU64::new(u64::MAX);

        extern "C" fn child() -> ! {
            let me = crate::sched::current_id();
            let ppid = crate::sched::parent_of(me).unwrap_or(u64::MAX);
            CHILD_PPID.store(ppid, Ordering::SeqCst);
            crate::sched::exit_current_with_code(0);
        }

        let me = crate::sched::current_id();
        let id = crate::sched::spawn("smoke_ppid", child);
        let _ = crate::sched::wait_for(id);
        let ppid = CHILD_PPID.load(Ordering::SeqCst);
        s.check("getppid_matches", ppid == me, "ppid != parent id");
    }

    // ---------- 10. FAT32 subdirs (soft) ----------
    {
        let entries = crate::vfs::fat32_list_root();
        if entries.is_empty() {
            s.skip("fat32_subdirs", "no FAT32 mounted");
        } else {
            let _ = crate::vfs::fat32_remove_path("C:/__smoke_tmp/a.txt");
            let _ = crate::vfs::fat32_remove_path("C:/__smoke_tmp");

            let mk = crate::vfs::fat32_mkdir_path("C:/__smoke_tmp");
            s.check("fat32_mkdir_path", mk, "mkdir не сработал");

            let wr = crate::vfs::fat32_write_path("C:/__smoke_tmp/a.txt", b"hello");
            s.check("fat32_write_subdir_file", wr, "write не сработал");

            let rd = crate::vfs::fat32_read_path("C:/__smoke_tmp/a.txt");
            s.check(
                "fat32_read_subdir_file",
                rd.as_deref() == Some(b"hello"),
                "read не совпал",
            );

            let st = crate::vfs::fat32_stat_path("C:/__smoke_tmp/a.txt");
            s.check("fat32_stat_file", st == Some((5, false)), "stat не совпал");

            let st_dir = crate::vfs::fat32_stat_path("C:/__smoke_tmp");
            s.check("fat32_stat_dir", st_dir == Some((0, true)), "stat(dir) не совпал");

            let _ = crate::vfs::fat32_remove_path("C:/__smoke_tmp/a.txt");
            let rm = crate::vfs::fat32_remove_path("C:/__smoke_tmp");
            s.check("fat32_remove_dir", rm, "remove(dir) не сработал");
        }
    }

    // ---------- 11. time_ms ----------
    {
        let t0 = crate::interrupts::ticks();
        let ms = (t0 * 1000) / 18;
        s.check("time_ms_no_overflow", ms < u64::MAX / 2, "переполнение");
    }

    serial_println!("[TEST] === smoke tests end: passed={} failed={} ===",
        s.passed, s.failed);
    s.failed
}