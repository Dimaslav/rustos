//! Smoke-тесты. Запускаются в `kernel_main` при `feature = "headless_test"`.

use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

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

    // ============================================================
    // 1. Базовая инфраструктура: heap, format, Vec, String
    // ============================================================
    {
        use alloc::vec::Vec;
        let v: Vec<u8> = (0..1000).map(|i| (i & 0xFF) as u8).collect();
        s.check("heap_alloc_1000",
            v.len() == 1000 && v[500] == ((500u32 & 0xFF) as u8),
            "Vec len or content mismatch");
    }
    {
        use alloc::format;
        let str_val: String = format!("{}-{}", 1, 2);
        s.check("format_basic", str_val == "1-2", "format! вернул не то");

        let mut v: Vec<u32> = Vec::new();
        for i in 0..100 { v.push(i); }
        let sum: u32 = v.iter().sum();
        s.check("vec_sum_4950", sum == 4950, "сумма != 4950");
    }
    {
        let mut blocks: Vec<Vec<u8>> = Vec::new();
        for i in 0..64 {
            let b = alloc::vec![(i & 0xFF) as u8; 100];
            blocks.push(b);
        }
        let all_ok = blocks.iter().enumerate().all(|(i, b)| {
            b.iter().all(|&x| x == (i as u8 & 0xFF))
        });
        s.check("heap_many_allocs_no_overlap", all_ok,
            "перекрытие или порча аллокаций");
    }

    // ============================================================
    // 2. RAMFS
    // ============================================================
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

        let _ = crate::vfs::ramfs_mkdir("/", "smoke_sub");
        let c = crate::vfs::ramfs_create_file("/smoke_sub", "a.txt", b"abc");
        let r = crate::vfs::ramfs_read("/smoke_sub/a.txt");
        s.check("ramfs_subdir_write_read",
            c && r.as_deref() == Some(b"abc"),
            "subdir write/read mismatch");
        let _ = crate::vfs::ramfs_remove("/smoke_sub/a.txt");
        let _ = crate::vfs::ramfs_remove("/smoke_sub");
    }

    // ============================================================
    // 3. VFS API
    // ============================================================
    {
        let list = crate::vfs::list("/").ok().unwrap_or_default();
        let has = list.iter().any(|e| e.name == "smokedir");
        s.check("vfs_list_root", has, "smokedir не видно в листинге /");
    }

    // ============================================================
    // 4. FAT32 (soft — диск может отсутствовать)
    // ============================================================
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

    // ============================================================
    // 5. Interrupts / ticks
    // ============================================================
    {
        let t0 = crate::interrupts::ticks();
        crate::sched::sleep_ms(100);
        let t1 = crate::interrupts::ticks();
        s.check("timer_advances", t1 > t0,
            "ticks не растут (IRQ0 не работает?)");
    }

    // ============================================================
    // 6. Scheduler: spawn + exit code + wait
    // ============================================================
    {
        extern "C" fn test_thread() -> ! {
            crate::sched::exit_current_with_code(42);
        }
        let id = crate::sched::spawn("smoke_child", test_thread);
        let code = crate::sched::wait_for(id);
        s.check("wait_exit_code_42", code == Some(42), "exit code не 42");
    }

    // ============================================================
    // 7. Scheduler: kill
    // ============================================================
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

    // ============================================================
    // 8. getppid
    // ============================================================
    {
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

    // ============================================================
    // 9. Sleep + wake_thread
    // ============================================================
    {
        static WOKEN: AtomicU64 = AtomicU64::new(0);
        extern "C" fn sleeper() -> ! {
            WOKEN.store(1, Ordering::SeqCst);
            crate::sched::sleep_forever();
            WOKEN.store(2, Ordering::SeqCst);
            crate::sched::exit_current_with_code(0);
        }
        let id = crate::sched::spawn("smoke_sleep", sleeper);
        crate::sched::sleep_ms(30);
        let before = WOKEN.load(Ordering::SeqCst);
        crate::sched::wake_thread(id);
        let _ = crate::sched::wait_for(id);
        let after = WOKEN.load(Ordering::SeqCst);
        s.check("sleep_and_wake",
            before == 1 && after == 2,
            "не разбудили или не доспал");
    }

    // ============================================================
    // 10. FAT32 subdirs (soft)
    // ============================================================
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
            s.check("fat32_read_subdir_file",
                rd.as_deref() == Some(b"hello"), "read не совпал");
            let st = crate::vfs::fat32_stat_path("C:/__smoke_tmp/a.txt");
            s.check("fat32_stat_file", st == Some((5, false)), "stat не совпал");
            let st_dir = crate::vfs::fat32_stat_path("C:/__smoke_tmp");
            s.check("fat32_stat_dir", st_dir == Some((0, true)), "stat(dir) не совпал");
            let _ = crate::vfs::fat32_remove_path("C:/__smoke_tmp/a.txt");
            let rm = crate::vfs::fat32_remove_path("C:/__smoke_tmp");
            s.check("fat32_remove_dir", rm, "remove(dir) не сработал");
        }
    }

    // ============================================================
    // 11. time_ms
    // ============================================================
    {
        let t0 = crate::interrupts::ticks();
        let ms = (t0 * 1000) / 18;
        s.check("time_ms_no_overflow", ms < u64::MAX / 2, "переполнение");
    }

    // ============================================================
    // 12. procfs
    // ============================================================
    {
        let version = crate::vfs::read("/proc/version").ok();
        s.check("procfs_version_read",
            version.map(|v| !v.is_empty()).unwrap_or(false),
            "/proc/version не читается");

        let entries = crate::vfs::list("/proc").ok().unwrap_or_default();
        let has_version = entries.iter().any(|e| e.name == "version");
        s.check("procfs_list_root", has_version, "version не виден в /proc");

        let st = crate::vfs::ramfs_stat("/proc");
        s.check("procfs_stat_root",
            st == Some((0, true)), "stat(/proc) != (0, true)");

        let mem = crate::vfs::read("/proc/meminfo").ok();
        s.check("procfs_meminfo",
            mem.map(|v| !v.is_empty()).unwrap_or(false),
            "/proc/meminfo не читается");

        let maps = crate::vfs::read("/proc/self/maps").ok();
        s.check("procfs_self_maps",
            maps.map(|v| !v.is_empty()).unwrap_or(false),
            "/proc/self/maps не читается");

        s.check("procfs_is_proc_path",
            crate::procfs::is_proc_path("/proc")
                && crate::procfs::is_proc_path("/proc/version")
                && !crate::procfs::is_proc_path("/usr")
                && !crate::procfs::is_proc_path("/procedure"),
            "is_proc_path false positive/negative");

        let ps = crate::vfs::read("/proc/ps").ok();
        let has_header = ps
            .as_ref()
            .map(|v| core::str::from_utf8(v).unwrap_or("").contains("PID"))
            .unwrap_or(false);
        s.check("procfs_ps_header", has_header, "/proc/ps без заголовка PID");
    }

    // ============================================================
    // 13. NX (No-Execute) включён
    // ============================================================
    {
        use x86_64::registers::model_specific::{Efer, EferFlags};
        let flags = Efer::read();
        s.check("nx_enabled",
            flags.contains(EferFlags::NO_EXECUTE_ENABLE),
            "EFER.NXE не выставлен");
    }

    // ============================================================
    // 14. Heap-страницы помечены NO_EXECUTE
    // ============================================================
    {
        use x86_64::structures::paging::Translate;
        use x86_64::VirtAddr;

        let phys = crate::memory::PHYS_OFFSET.load(Ordering::Acquire);
        if phys == 0 {
            s.fail("heap_pages_are_nx", "phys_offset=0");
        } else {
            let addr = VirtAddr::new(crate::allocator::HEAP_START as u64);
            let (pml4_frame, _) = x86_64::registers::control::Cr3::read();
            let pml4_virt = VirtAddr::new(phys + pml4_frame.start_address().as_u64());
            let pml4: *const u64 = pml4_virt.as_ptr();
            let i4 = ((addr.as_u64() >> 39) & 0x1FF) as usize;
            let e4 = unsafe { *pml4.add(i4) };
            let pdpt_phys = e4 & 0x000F_FFFF_FFFF_F000;
            let pdpt_virt = VirtAddr::new(phys + pdpt_phys);
            let i3 = ((addr.as_u64() >> 30) & 0x1FF) as usize;
            let e3 = unsafe { *(pdpt_virt.as_ptr::<u64>().add(i3)) };
            let pd_phys = e3 & 0x000F_FFFF_FFFF_F000;
            let pd_virt = VirtAddr::new(phys + pd_phys);
            let i2 = ((addr.as_u64() >> 21) & 0x1FF) as usize;
            let e2 = unsafe { *(pd_virt.as_ptr::<u64>().add(i2)) };
            let pt_phys = e2 & 0x000F_FFFF_FFFF_F000;
            let pt_virt = VirtAddr::new(phys + pt_phys);
            let i1 = ((addr.as_u64() >> 12) & 0x1FF) as usize;
            let e1 = unsafe { *(pt_virt.as_ptr::<u64>().add(i1)) };
            let nx = e1 & (1u64 << 63) != 0;
            s.check("heap_pages_are_nx", nx,
                "heap-страница не помечена NO_EXECUTE");

            let mapped = unsafe {
                use x86_64::structures::paging::OffsetPageTable;
                let l4 = &mut *(pml4_virt
                    .as_mut_ptr::<x86_64::structures::paging::PageTable>());
                let mapper = OffsetPageTable::new(l4, VirtAddr::new(phys));
                mapper.translate_addr(addr).is_some()
            };
            s.check("heap_page_mapped", mapped, "heap-страница не отображена");
        }
    }

    // ============================================================
    // 15. USER_ADDR_MAX sanity
    // ============================================================
    {
        const EXPECTED: u64 = 0x0000_8000_0000_0000;
        s.check("user_addr_max_constant",
            crate::memory::USER_ADDR_MAX == EXPECTED,
            "USER_ADDR_MAX изменился — проверь весь код");
    }

    // ============================================================
    // 16. is_user_range_mapped — в kernel-AS должен быть false
    // ============================================================
    {
        let r = unsafe { crate::memory::is_user_range_mapped(0x400000, 16) };
        s.check("user_range_not_mapped_in_kernel_as", !r,
            "user-адрес 0x400000 считается отображённым в kernel-AS");
    }

    // ============================================================
    // 17. AddressSpace new/destroy
    // ============================================================
    {
        use x86_64::VirtAddr;

        let phys = crate::memory::PHYS_OFFSET.load(Ordering::Acquire);
        let created = if phys == 0 {
            false
        } else {
            // ИСПРАВЛЕНИЕ: лок FRAME_ALLOCATOR нельзя держать во время
            // destroy — destroy сам берёт этот же лок, получается deadlock.
            // Аллоцируем AS в узком блоке, отпускаем лок, потом destroy.
            let pml4_opt = {
                let mut guard = crate::memory::FRAME_ALLOCATOR.lock();
                if let Some(alloc) = guard.as_mut() {
                    unsafe {
                        crate::memory::AddressSpace::new_user(VirtAddr::new(phys), alloc)
                    }.map(|a| a.pml4_frame.start_address().as_u64())
                } else {
                    None
                }
            };
            match pml4_opt {
                Some(pml4) => {
                    unsafe { crate::memory::AddressSpace::destroy(pml4); }
                    true
                }
                None => false,
            }
        };
        s.check("address_space_create_destroy", created,
            "не удалось создать/удалить AS");
    }

    // ============================================================
    // 18. pipe — создание и обмен
    // ============================================================
    {
        if let Some(id) = crate::pipe::create() {
            let alive = crate::pipe::is_alive(id);
            let mut out = [0u8; 8];
            let n = crate::pipe::write(id, b"abc").unwrap_or(0);
            let r = crate::pipe::read(id, &mut out).unwrap_or(0);
            s.check("pipe_write_read",
                alive && n == 3 && r == 3 && &out[..3] == b"abc",
                "pipe write/read mismatch");
            crate::pipe::close_reader(id);
            crate::pipe::close_writer(id);
        } else {
            s.fail("pipe_write_read", "pipe::create вернул None");
        }
    }

    // ============================================================
    // 19. fd table — alloc, dup2, close
    // ============================================================
    {
        use crate::fd::{self, FdKind};
        let pid = crate::sched::current_id();

        fd::init_for_pid(pid);
        let have_stdin = fd::get(pid, 0).is_some();

        let fd_no = fd::alloc_fd(pid, FdKind::Stdin);
        let got = fd::get(pid, fd_no).is_some();
        let dup_ok = fd::dup2(pid, 0, 200);
        let dup_kind = fd::get(pid, 200).is_some();
        let closed = fd::close(pid, fd_no);
        let _ = fd::close(pid, 200);

        s.check("fd_alloc_dup_close",
            have_stdin && got && dup_ok && dup_kind && closed,
            "fd operations не прошли");
    }

    // ============================================================
    // 20. procfs::read синтаксис meminfo
    // ============================================================
    {
        let mem = crate::vfs::read("/proc/meminfo").ok().unwrap_or_default();
        let s_str = core::str::from_utf8(&mem).unwrap_or("");
        s.check("procfs_meminfo_format",
            s_str.contains("HeapTotal") && s_str.contains("HeapUsed"),
            "/proc/meminfo не содержит ожидаемых полей");
    }

    // ============================================================
    // 21. /proc/self/status содержит Pid:
    // ============================================================
    {
        let st = crate::vfs::read("/proc/self/status").ok().unwrap_or_default();
        let s_str = core::str::from_utf8(&st).unwrap_or("");
        s.check("procfs_self_status_format",
            s_str.contains("Pid:") && s_str.contains("Name:"),
            "/proc/self/status без Pid/Name");
    }

    // ============================================================
    // 22. procfs list имеет self/ как директорию
    // ============================================================
    {
        let entries = crate::vfs::list("/proc").ok().unwrap_or_default();
        let self_entry = entries.iter().find(|e| e.name == "self");
        s.check("procfs_self_is_dir",
            self_entry.map(|e| e.is_dir).unwrap_or(false),
            "self не директория в /proc");
    }

    // ============================================================
    // 23. Efer::read/write не портит флаги
    // ============================================================
    {
        use x86_64::registers::model_specific::{Efer, EferFlags};
        let before = Efer::read();
        let has_nxe = before.contains(EferFlags::NO_EXECUTE_ENABLE);
        let has_lme = before.contains(EferFlags::LONG_MODE_ENABLE);
        s.check("efer_flags_intact", has_nxe && has_lme,
            "EFER потерял NXE или LME");
    }

    // ============================================================
    // 24. FAT32 fs — корректный stat C:/ (если смонтирован)
    // ============================================================
    {
        let entries = crate::vfs::fat32_list_root();
        if entries.is_empty() {
            s.skip("fat32_root_stat", "no FAT32 mounted");
        } else {
            let st = crate::vfs::fat32_stat_path("C:/");
            s.check("fat32_root_stat",
                st.is_none() || st == Some((0, true)),
                "stat(C:/) неожиданный");
        }
    }

    // ============================================================
    // 25. FAT32 LFN — запись и чтение длинного имени
    // ============================================================
    {
        let entries = crate::vfs::fat32_list_root();
        if entries.is_empty() {
            s.skip("fat32_lfn_roundtrip", "no FAT32 mounted");
        } else {
            let name = "very_long_filename_тест.txt";
            let _ = crate::vfs::fat32_remove_path(&alloc::format!("C:/{}", name));
            let wr = crate::vfs::fat32_write_path(
                &alloc::format!("C:/{}", name), b"lfn-data");
            let rd = crate::vfs::fat32_read_path(
                &alloc::format!("C:/{}", name));
            let found = crate::vfs::fat32_list_root()
                .into_iter()
                .any(|(n, _, _)| n == name);
            let _ = crate::vfs::fat32_remove_path(&alloc::format!("C:/{}", name));
            s.check("fat32_lfn_roundtrip",
                wr && rd.as_deref() == Some(b"lfn-data") && found,
                "LFN write/read/list mismatch");
        }
    }

    // ============================================================
    // 26. Список потоков
    // ============================================================
    {
        let threads = crate::sched::list_threads_snapshot();
        let has_main = threads.iter().any(|t| {
            let end = t.name.iter().position(|&b| b == 0).unwrap_or(32);
            core::str::from_utf8(&t.name[..end]).unwrap_or("") == "main"
        });
        s.check("list_threads_has_main", has_main,
            "в снапшоте нет main-потока");
        s.check("list_threads_non_empty", !threads.is_empty(),
            "снапшот пуст");
    }

    // ============================================================
    // 27. Системные тики не сбрасываются
    // ============================================================
    {
        let t0 = crate::interrupts::ticks();
        crate::sched::sleep_ms(200);
        let t1 = crate::interrupts::ticks();
        s.check("ticks_monotonic", t1 >= t0 && t1 - t0 >= 3,
            "ticks не растут за 200мс (ожидали хотя бы 3 тика)");
    }

    // ============================================================
    // 28. procfs не пишется
    // ============================================================
    {
        let w = crate::vfs::write("/proc/version", b"hacked");
        s.check("procfs_readonly", w.is_err(),
            "write в /proc прошёл");
    }

    // ============================================================
    // 29. procfs не создаёт файлов
    // ============================================================
    {
        let c = crate::vfs::ramfs_create_file("/proc", "fake", b"x");
        s.check("procfs_no_create_in", !c,
            "create_file в /proc прошёл");
    }

    // ============================================================
    // 30. Ticks после стольких тестов всё ещё идут
    // ============================================================
    {
        let a = crate::interrupts::ticks();
        crate::sched::sleep_ms(50);
        let b = crate::interrupts::ticks();
        s.check("timer_alive_at_end", b > a,
            "таймер умер во время тестов");
    }

    // ============================================================
    serial_println!(
        "[TEST] === smoke tests end: passed={} failed={} ===",
        s.passed, s.failed
    );
    s.failed
}