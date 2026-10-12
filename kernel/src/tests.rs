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

    // ---------- 1. Базовая инфраструктура ----------
    {
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
    {
        let layout = core::alloc::Layout::from_size_align(13, 64).unwrap();
        let p = unsafe { alloc::alloc::alloc(layout) };
        let ok = !p.is_null() && (p as usize) % 64 == 0;
        if !p.is_null() { unsafe { alloc::alloc::dealloc(p, layout) }; }
        s.check("heap_align_64", ok, "аллокация не выровнена на 64");
    }

    // ---------- 2. RAMFS ----------
    {
        let created = crate::vfs::ramfs_create_file("/", "smoke.txt", b"hello");
        s.check("ramfs_create_file", created, "create_file вернул false");
        let data = crate::vfs::ramfs_read("/smoke.txt");
        s.check("ramfs_read_back",
            data.as_deref() == Some(b"hello"), "содержимое не совпало");
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
            c && r.as_deref() == Some(b"abc"), "subdir mismatch");
        let _ = crate::vfs::ramfs_remove("/smoke_sub/a.txt");
        let _ = crate::vfs::ramfs_remove("/smoke_sub");
    }
    {
        let _ = crate::vfs::ramfs_mkdir("/", "deep_a");
        let _ = crate::vfs::ramfs_mkdir("/deep_a", "deep_b");
        let _ = crate::vfs::ramfs_mkdir("/deep_a/deep_b", "deep_c");
        let c = crate::vfs::ramfs_create_file(
            "/deep_a/deep_b/deep_c", "f.txt", b"deep-content");
        let r = crate::vfs::ramfs_read("/deep_a/deep_b/deep_c/f.txt");
        s.check("ramfs_deep_paths",
            c && r.as_deref() == Some(b"deep-content"),
            "deep nesting не работает");
        let _ = crate::vfs::ramfs_remove("/deep_a/deep_b/deep_c/f.txt");
        let _ = crate::vfs::ramfs_remove("/deep_a/deep_b/deep_c");
        let _ = crate::vfs::ramfs_remove("/deep_a/deep_b");
        let _ = crate::vfs::ramfs_remove("/deep_a");
    }
    {
        let _ = crate::vfs::ramfs_create_file("/", "old_name.txt", b"rn");
        let rn = crate::vfs::ramfs_rename("/old_name.txt", "new_name.txt");
        let old_gone = !crate::vfs::ramfs_exists("/old_name.txt");
        let new_there = crate::vfs::ramfs_exists("/new_name.txt");
        let data = crate::vfs::ramfs_read("/new_name.txt");
        s.check("ramfs_rename",
            rn && old_gone && new_there && data.as_deref() == Some(b"rn"),
            "rename не сработал");
        let _ = crate::vfs::ramfs_remove("/new_name.txt");
    }

    // ---------- 3. VFS API ----------
    {
        let list = crate::vfs::list("/").ok().unwrap_or_default();
        let has = list.iter().any(|e| e.name == "smokedir");
        s.check("vfs_list_root", has, "smokedir не видно в листинге /");
    }
    {
        let _ = crate::vfs::ramfs_create_file("/", "vfs_rt.txt", b"");
        let w = crate::vfs::write("/vfs_rt.txt", b"vfs-data");
        let r = crate::vfs::read("/vfs_rt.txt");
        s.check("vfs_write_read_ramfs",
            w.is_ok() && r.as_deref() == Ok(&b"vfs-data"[..]),
            "vfs write/read mismatch");
        let _ = crate::vfs::ramfs_remove("/vfs_rt.txt");
    }
    {
        let _ = crate::vfs::ramfs_create_file("/", "vfs_ex.txt", b"x");
        let ok = crate::vfs::exists("/vfs_ex.txt") && !crate::vfs::exists("/nope.txt");
        s.check("vfs_exists_ramfs", ok, "exists false positive/negative");
        let _ = crate::vfs::ramfs_remove("/vfs_ex.txt");
    }

    // ---------- 4. FAT32 (soft) ----------
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

    // ---------- 5. Interrupts ----------
    {
        let t0 = crate::interrupts::ticks();
        crate::sched::sleep_ms(100);
        let t1 = crate::interrupts::ticks();
        s.check("timer_advances", t1 > t0, "ticks не растут");
    }
    {
        let t0 = crate::interrupts::ticks();
        crate::sched::sleep_ms(500);
        let t1 = crate::interrupts::ticks();
        s.check("ticks_after_long_sleep",
            t1.saturating_sub(t0) >= 5,
            "за 500мс прошло меньше 5 тиков");
    }

    // ---------- 6. Scheduler ----------
    {
        extern "C" fn test_thread() -> ! {
            crate::sched::exit_current_with_code(42);
        }
        let id = crate::sched::spawn("smoke_child", test_thread);
        let code = crate::sched::wait_for(id);
        s.check("wait_exit_code_42", code == Some(42), "exit code не 42");
    }
    {
        extern "C" fn slow_thread() -> ! {
            loop { crate::sched::sleep_ms(1000); }
        }
        let id = crate::sched::spawn("smoke_kill", slow_thread);
        crate::sched::sleep_ms(50);
        let killed = crate::sched::kill(id);
        s.check("kill_returns_true", killed, "kill вернул false");
        let code = crate::sched::wait_for(id);
        s.check("kill_exit_code_neg", code == Some(-9), "ожидали -9");
    }
    {
        extern "C" fn slow2() -> ! {
            loop { crate::sched::sleep_ms(1000); }
        }
        let id = crate::sched::spawn("smoke_kill_fast", slow2);
        let killed = crate::sched::kill(id);
        let code = crate::sched::wait_for(id);
        s.check("spawn_and_immediate_kill",
            killed && code == Some(-9), "immediate kill mismatch");
    }
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
            before == 1 && after == 2, "не разбудили или не доспал");
    }
    {
        extern "C" fn blocked() -> ! {
            crate::sched::sleep_forever();
            loop { crate::sched::sleep_ms(1000); }
        }
        let id = crate::sched::spawn("smoke_snapshot", blocked);
        crate::sched::sleep_ms(20);
        let snap = crate::sched::list_threads_snapshot();
        let found = snap.iter().any(|t| t.id == id);
        crate::sched::kill(id);
        let _ = crate::sched::wait_for(id);
        s.check("spawn_thread_state_snapshot", found,
            "поток не виден в snapshot");
    }

    // ---------- 7. FAT32 subdirs (soft) ----------
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

    // ---------- 8. time_ms ----------
    {
        let t0 = crate::interrupts::ticks();
        let ms = (t0 * 1000) / 18;
        s.check("time_ms_no_overflow", ms < u64::MAX / 2, "переполнение");
    }

    // ---------- 9. procfs ----------
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
    {
        let up = crate::vfs::read("/proc/uptime").ok().unwrap_or_default();
        let s_str = core::str::from_utf8(&up).unwrap_or("");
        s.check("procfs_uptime_format",
            s_str.contains('.') && s_str.ends_with('\n'),
            "/proc/uptime не содержит точку или \\n");
    }
    {
        let mem = crate::vfs::read("/proc/meminfo").ok().unwrap_or_default();
        let s_str = core::str::from_utf8(&mem).unwrap_or("");
        s.check("procfs_meminfo_format",
            s_str.contains("HeapTotal") && s_str.contains("HeapUsed"),
            "/proc/meminfo не содержит полей");
    }
    {
        let st = crate::vfs::read("/proc/self/status").ok().unwrap_or_default();
        let s_str = core::str::from_utf8(&st).unwrap_or("");
        s.check("procfs_self_status_format",
            s_str.contains("Pid:") && s_str.contains("Name:"),
            "/proc/self/status без Pid/Name");
    }
    {
        let entries = crate::vfs::list("/proc").ok().unwrap_or_default();
        let self_entry = entries.iter().find(|e| e.name == "self");
        s.check("procfs_self_is_dir",
            self_entry.map(|e| e.is_dir).unwrap_or(false),
            "self не директория в /proc");
    }
    {
        let w = crate::vfs::write("/proc/version", b"hacked");
        s.check("procfs_readonly", w.is_err(), "write в /proc прошёл");
    }
    {
        let c = crate::vfs::ramfs_create_file("/proc", "fake", b"x");
        s.check("procfs_no_create_in", !c, "create_file в /proc прошёл");
    }

    // ---------- 10. NX ----------
    {
        use x86_64::registers::model_specific::{Efer, EferFlags};
        let flags = Efer::read();
        s.check("nx_enabled",
            flags.contains(EferFlags::NO_EXECUTE_ENABLE),
            "EFER.NXE не выставлен");
        s.check("efer_flags_intact",
            flags.contains(EferFlags::LONG_MODE_ENABLE)
                && flags.contains(EferFlags::NO_EXECUTE_ENABLE),
            "EFER потерял NXE или LME");
    }

    // ---------- 11. Heap-страницы NX ----------
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
            let pdpt_virt = VirtAddr::new(phys + (e4 & 0x000F_FFFF_FFFF_F000));
            let i3 = ((addr.as_u64() >> 30) & 0x1FF) as usize;
            let e3 = unsafe { *(pdpt_virt.as_ptr::<u64>().add(i3)) };
            let pd_virt = VirtAddr::new(phys + (e3 & 0x000F_FFFF_FFFF_F000));
            let i2 = ((addr.as_u64() >> 21) & 0x1FF) as usize;
            let e2 = unsafe { *(pd_virt.as_ptr::<u64>().add(i2)) };
            let pt_virt = VirtAddr::new(phys + (e2 & 0x000F_FFFF_FFFF_F000));
            let i1 = ((addr.as_u64() >> 12) & 0x1FF) as usize;
            let e1 = unsafe { *(pt_virt.as_ptr::<u64>().add(i1)) };
            let nx = e1 & (1u64 << 63) != 0;
            s.check("heap_pages_are_nx", nx, "heap-страница не NX");

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

    // ---------- 12. Константы ----------
    {
        const EXPECTED: u64 = 0x0000_8000_0000_0000;
        s.check("user_addr_max_constant",
            crate::memory::USER_ADDR_MAX == EXPECTED,
            "USER_ADDR_MAX изменился");
        s.check("user_pml4_index_zero",
            crate::memory::USER_PML4_INDEX == 0,
            "USER_PML4_INDEX != 0");
    }

    // ---------- 13. is_user_range_mapped ----------
    {
        let in_kernel_as = unsafe { crate::memory::is_user_range_mapped(0x400000, 16) };
        s.check("user_range_not_mapped_in_kernel_as", !in_kernel_as,
            "user-адрес считается отображённым в kernel-AS");

        let null_zero = unsafe { crate::memory::is_user_range_mapped(0, 0) };
        s.check("is_user_range_null_zero_len", null_zero,
            "нулевой ptr с нулевой длиной должен быть ok");

        let null_nonzero = unsafe { crate::memory::is_user_range_mapped(0, 8) };
        s.check("is_user_range_null_nonzero_len", !null_nonzero,
            "нулевой ptr с ненулевой длиной должен быть отвергнут");

        let overflow = unsafe {
            crate::memory::is_user_range_mapped(0x7FFF_FFFF_FFF0, 0x20)
        };
        s.check("is_user_range_no_overflow", !overflow,
            "overflow не пойман");
    }

    // ---------- 14. AddressSpace ----------
    {
        use x86_64::VirtAddr;
        let phys = crate::memory::PHYS_OFFSET.load(Ordering::Acquire);
        let created = if phys == 0 {
            false
        } else {
            let pml4_opt = {
                let mut guard = crate::memory::FRAME_ALLOCATOR.lock();
                if let Some(alloc) = guard.as_mut() {
                    unsafe {
                        crate::memory::AddressSpace::new_user(VirtAddr::new(phys), alloc)
                    }.map(|a| a.pml4_frame.start_address().as_u64())
                } else { None }
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

    // ---------- 15. clone_user_pages (eager) в kernel-AS ----------
    {
        use x86_64::VirtAddr;
        let phys = crate::memory::PHYS_OFFSET.load(Ordering::Acquire);
        let ok = if phys == 0 {
            false
        } else {
            let mut guard = crate::memory::FRAME_ALLOCATOR.lock();
            let alloc = match guard.as_mut() { Some(a) => a, None => return 1 };
            let parent = crate::sched::current_pml4();
            match parent {
                Some(p) => unsafe {
                    let child = match crate::memory::AddressSpace::new_user(
                        VirtAddr::new(phys), alloc)
                    {
                        Some(a) => a.pml4_frame.start_address().as_u64(),
                        None => { return 1; }
                    };
                    let r = crate::memory::clone_user_pages(
                        p, child, VirtAddr::new(phys), alloc);
                    crate::memory::AddressSpace::destroy(child);
                    r
                },
                None => true,
            }
        };
        s.check("clone_user_pages_kernel_as", ok,
            "clone_user_pages упал в kernel-AS");
    }

    // ---------- 16. fork_in_place отвергает kernel-поток ----------
    {
        let mut fake_regs = [0u64; 21];
        fake_regs[16] = 0;
        fake_regs[19] = 0;
        let r = unsafe { crate::user::fork_in_place(fake_regs.as_mut_ptr()) };
        s.check("fork_rejects_non_user", r.is_err(),
            "fork_in_place принял не-user поток");
    }

    // ---------- 17. user_mappings_snapshot ----------
    {
        let snap = unsafe { crate::memory::user_mappings_snapshot() };
        serial_println!("[TEST INFO] kernel-AS user_mappings: {} entries", snap.len());
        s.pass("user_mappings_snapshot_kernel");
    }

    // ---------- 18. heap accounting ----------
    {
        let (used, free, total) = {
            let a = crate::allocator::ALLOCATOR.lock();
            (a.used(), a.free(), crate::allocator::HEAP_SIZE)
        };
        s.check("heap_used_plus_free",
            used + free == total,
            "used + free != HEAP_SIZE");
    }

    // ---------- 19. pipe ----------
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
    {
        if let Some(id) = crate::pipe::create() {
            let data = [0u8; 5000];
            let first = crate::pipe::write(id, &data);
            let second = crate::pipe::write(id, &data);
            s.check("pipe_capacity_full",
                first == Some(4096) && second.is_none(),
                "pipe capacity mismatch");
            crate::pipe::close_reader(id);
            crate::pipe::close_writer(id);
        } else {
            s.fail("pipe_capacity_full", "pipe::create вернул None");
        }
    }
    {
        if let Some(id) = crate::pipe::create() {
            crate::pipe::close_writer(id);
            let mut buf = [0u8; 8];
            let r = crate::pipe::read(id, &mut buf);
            s.check("pipe_writer_close_eof", r == Some(0),
                "read после close_writer не вернул EOF");
            crate::pipe::close_reader(id);
        } else {
            s.fail("pipe_writer_close_eof", "pipe::create вернул None");
        }
    }

    // ---------- 20. fd ----------
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
    {
        use crate::fd;
        let parent = crate::sched::current_id();
        fd::init_for_pid(parent);
        let fake_child: u64 = 999_999;
        fd::fork_from(parent, fake_child);
        let has_stdin  = fd::get(fake_child, 0).is_some();
        let has_stdout = fd::get(fake_child, 1).is_some();
        let has_stderr = fd::get(fake_child, 2).is_some();
        fd::close_all_for_pid(fake_child);
        s.check("fd_fork_from_copies_std",
            has_stdin && has_stdout && has_stderr,
            "fork_from не скопировал std fd");
    }

    // ---------- 21. FAT32 root stat ----------
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

    // ---------- 22. FAT32 LFN ----------
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

    // ---------- 23. Список потоков ----------
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

    // ---------- 24. Ticks monotonic ----------
    {
        let t0 = crate::interrupts::ticks();
        crate::sched::sleep_ms(200);
        let t1 = crate::interrupts::ticks();
        s.check("ticks_monotonic", t1 >= t0 && t1 - t0 >= 3,
            "ticks не растут за 200мс");
    }

    // ---------- 25. Timer alive ----------
    {
        let a = crate::interrupts::ticks();
        crate::sched::sleep_ms(50);
        let b = crate::interrupts::ticks();
        s.check("timer_alive_at_end", b > a, "таймер умер во время тестов");
    }

    // ---------- 26. current_pml4 в kernel main ----------
    {
        let p = crate::sched::current_pml4();
        s.check("current_pml4_is_none_in_main",
            p.is_none(),
            "current_pml4 не None в kernel main");
    }

    // ---------- 27. COW infrastructure ----------
    {
        // cow_release на неразделяемом фрейме → true.
        let released = crate::memory::cow_release(0xDEAD_BEEF_000);
        s.check("cow_release_unshared", released, "unshared release != true");

        // cow_mark_shared дважды → refcount 3, освобождается только на 3-й.
        crate::memory::cow_mark_shared(0xCAFE_0000);
        crate::memory::cow_mark_shared(0xCAFE_0000);
        let r1 = crate::memory::cow_release(0xCAFE_0000);
        let r2 = crate::memory::cow_release(0xCAFE_0000);
        let r3 = crate::memory::cow_release(0xCAFE_0000);
        s.check("cow_refcount_lifecycle",
            !r1 && !r2 && r3,
            "refcount списывается неправильно");
    }

    // ----------
    serial_println!(
        "[TEST] === smoke tests end: passed={} failed={} ===",
        s.passed, s.failed
    );
    s.failed
}