use std::path::PathBuf;
use std::process::Command;

fn main() {
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let kernel_dir = PathBuf::from("kernel");
    let kernel_target = "x86_64-unknown-none";

    // Собираем ядро в release — для framebuffer-рендерера это критично.
    let status = Command::new("cargo")
        .current_dir(&kernel_dir)
        .args([
            "build",
            "--release",
            "--target", kernel_target,
            "-Zbuild-std=core,alloc,compiler_builtins",
            "-Zbuild-std-features=compiler-builtins-mem",
        ])
        .status()
        .expect("Не удалось запустить сборку ядра");

    if !status.success() {
        panic!("Сборка ядра завершилась с ошибкой");
    }

    let release_dir = kernel_dir
        .join("target")
        .join(kernel_target)
        .join("release");

    let kernel_path = ["kernel", "kernel.exe"]
        .iter()
        .map(|name| release_dir.join(name))
        .find(|p| p.exists())
        .unwrap_or_else(|| {
            eprintln!("Содержимое {}:", release_dir.display());
            if let Ok(entries) = std::fs::read_dir(&release_dir) {
                for e in entries.flatten() {
                    eprintln!("  {}", e.path().display());
                }
            }
            panic!("Не найден бинарник ядра в {}", release_dir.display());
        });

    eprintln!("Найдено ядро: {}", kernel_path.display());

    let bios_path = out_dir.join("bios.img");
    bootloader::BiosBoot::new(&kernel_path)
        .create_disk_image(&bios_path)
        .expect("Не удалось создать образ");

    println!("cargo:rustc-env=BIOS_PATH={}", bios_path.display());

    println!("cargo:rerun-if-changed=kernel");
}