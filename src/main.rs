use std::process::Command;

fn main() {
    let bios_path = env!("BIOS_PATH");

    let status = Command::new("qemu-system-x86_64")
        .args([
            "-drive", &format!("format=raw,file={}", bios_path),
            "-display", "gtk",
        ])
        .status()
        .expect("QEMU не найден. Установите qemu-system-x86_64 и добавьте в PATH.");

    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
}