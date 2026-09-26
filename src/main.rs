use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let headless = args.iter().any(|a| a == "--test");

    let bios_path = env!("BIOS_PATH");
    let disk_path = "fat32.img";

    if !Path::new(disk_path).exists() {
        try_make_disk();
    }

    if headless {
        run_headless(bios_path, disk_path);
    } else {
        run_interactive(bios_path, disk_path);
    }
}

fn try_make_disk() {
    for py in ["python", "python3"] {
        if Command::new(py).arg("--version").output().is_ok() {
            eprintln!("[myos] fat32.img не найден — запускаю {} make_disk.py", py);
            let status = Command::new(py).arg("make_disk.py").status();
            if let Ok(s) = status {
                if s.success() { return; }
            }
        }
    }
    eprintln!("[myos] WARN: не удалось создать fat32.img. Продолжаю без диска.");
}

fn which_qemu() -> &'static str {
    for name in ["qemu-system-x86_64", "qemu-system-x86_64.exe"] {
        if Command::new(name).arg("--version").output().is_ok() {
            return name;
        }
    }
    eprintln!("[myos] qemu-system-x86_64 не найден в PATH");
    std::process::exit(1);
}

fn run_interactive(bios_path: &str, disk_path: &str) {
    let qemu = which_qemu();
    let status = Command::new(qemu)
        .args([
            "-m", "512M",
            "-drive", &format!("format=raw,file={}", bios_path),
            "-drive", &format!("format=raw,file={},if=ide,index=1,media=disk", disk_path),
            "-serial", "stdio",
            "-no-reboot",
            "-no-shutdown",
            // VGA с расширенной памятью + EDID 1920x1080.
            // Через -global, а не -device: QEMU 11.x корректно применяет
            // эти свойства к дефолтному VGA, даже если создаётся -vga std.
            "-vga", "std",
            "-global", "VGA.vgamem_mb=64",
            "-global", "VGA.edid=on",
            "-global", "VGA.xres=1920",
            "-global", "VGA.yres=1080",
            "-display", "gtk,zoom-to-fit=on",
        ])
        .status()
        .expect("QEMU не запустился");

    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
}

fn run_headless(bios_path: &str, disk_path: &str) {
    let qemu = which_qemu();
    let log = "serial.log";
    let _ = std::fs::remove_file(log);

    let mut cmd = Command::new(qemu);
    cmd.args([
        "-m", "512M",
        "-drive", &format!("format=raw,file={}", bios_path),
        "-serial", &format!("file:{}", log),
        "-display", "none",
        "-no-reboot",
        "-vga", "std",
        "-global", "VGA.vgamem_mb=32",
    ]);
    if Path::new(disk_path).exists() {
        cmd.args([
            "-drive",
            &format!("format=raw,file={},if=ide,index=1,media=disk", disk_path),
        ]);
    }

    eprintln!("[test] запускаю QEMU headless (timeout=30s)");
    let mut child = cmd.spawn().expect("QEMU не запустился");

    let timeout = Duration::from_secs(30);
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                eprintln!("[test] QEMU exited: code={:?}", status.code());
                break;
            }
            Ok(None) => {
                if start.elapsed() > timeout {
                    eprintln!("[test] timeout — killing QEMU");
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                eprintln!("[test] try_wait error: {}", e);
                let _ = child.kill();
                break;
            }
        }
    }

    let content = std::fs::read_to_string(log).unwrap_or_default();

    println!();
    println!("[test] ==== serial.log (последние 40 строк) ====");
    let lines: Vec<&str> = content.lines().collect();
    let start_idx = lines.len().saturating_sub(40);
    for line in &lines[start_idx..] {
        println!("{}", line);
    }
    println!("[test] =========================================");
    println!();

    if content.contains("SMOKE_TEST_PASS") {
        println!("[test] RESULT: PASS");
        std::process::exit(0);
    } else if content.contains("SMOKE_TEST_FAIL") {
        println!("[test] RESULT: FAIL");
        std::process::exit(1);
    } else if content.contains("PANIC") {
        println!("[test] RESULT: PANIC");
        std::process::exit(2);
    } else {
        println!("[test] RESULT: NO RESULT (QEMU не успел или тесты не запустились)");
        std::process::exit(3);
    }
}