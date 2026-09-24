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
            "-Zbuild-std=core,compiler_builtins",
            "-Zbuild-std-features=compiler-builtins-mem",
        ])
        .status()
        .expect("Не удалось запустить сборку ядра");
    
    if !status.success() {
        panic!("Сборка ядра завершилась с ошибкой");
    }
    
    // 2. Путь к собранному ядру (теперь внутри kernel/target/)
    let kernel_path = kernel_dir
        .join("target")
        .join(kernel_target)
        .join("debug")
        .join("kernel.exe"); // на Windows — .exe

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