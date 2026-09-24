use std::path::PathBuf;
use std::process::Command;

fn main() {
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let kernel_dir = PathBuf::from("kernel");
    let kernel_target = "x86_64-unknown-none";

    // 1. Собираем ядро как отдельный крейт
    let status = Command::new("cargo")
        .current_dir(&kernel_dir)
        .args([
            "build",
            "--target", kernel_target,
            "-Zbuild-std=core,alloc,compiler_builtins",
            "-Zbuild-std-features=compiler-builtins-mem",
        ])
        .status()
        .expect("Не удалось запустить сборку ядра");

    if !status.success() {
        panic!("Сборка ядра завершилась с ошибкой");
    }

    // 2. Ищем бинарник — пробуем оба возможных имени
    let debug_dir = kernel_dir
        .join("target")
        .join(kernel_target)
        .join("debug");

    let kernel_path = ["kernel", "kernel.exe"]
        .iter()
        .map(|name| debug_dir.join(name))
        .find(|p| p.exists())
        .unwrap_or_else(|| {
            eprintln!("Содержимое {}:", debug_dir.display());
            if let Ok(entries) = std::fs::read_dir(&debug_dir) {
                for e in entries.flatten() {
                    eprintln!("  {}", e.path().display());
                }
            }
            panic!("Не найден бинарник ядра в {}", debug_dir.display());
        });

    eprintln!("Найдено ядро: {}", kernel_path.display());

    // 3. Создаём загрузочный образ
    let bios_path = out_dir.join("bios.img");
    bootloader::BiosBoot::new(&kernel_path)
        .create_disk_image(&bios_path)
        .expect("Не удалось создать образ");

    // 4. Передаём путь в main.rs
    println!("cargo:rustc-env=BIOS_PATH={}", bios_path.display());

    println!("cargo:rerun-if-changed=kernel/src");
    println!("cargo:rerun-if-changed=kernel/Cargo.toml");
}