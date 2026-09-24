use std::path::Path;
use std::process::Command;

fn main() {
    let bios_path = env!("BIOS_PATH");
    let disk_path = "fat32.img";

    if !Path::new(disk_path).exists() {
        eprintln!("Ошибка: {} не найден.", disk_path);
        eprintln!("Запустите один раз:");
        eprintln!("    python make_disk.py");
        std::process::exit(1);
    }

    let status = Command::new("qemu-system-x86_64")
        .args([
            "-drive", &format!("format=raw,file={}", bios_path),
            "-drive", &format!("format=raw,file={},if=ide,index=1,media=disk", disk_path),
            "-serial", "stdio",
            "-no-reboot",
            "-no-shutdown",
            "-vga", "std",
            "-global", "VGA.vgamem_mb=32",
            "-display", "gtk",
        ])
        .status()
        .expect("QEMU не найден.");

    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
}