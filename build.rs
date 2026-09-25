use std::path::PathBuf;
use std::process::Command;

fn main() {
    build_user_programs();

    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let kernel_dir = PathBuf::from("kernel");
    let kernel_target = "x86_64-unknown-none";

    let status = Command::new("cargo")
        .current_dir(&kernel_dir)
        .args([
            "build", "--release", "--target", kernel_target,
            "-Zbuild-std=core,alloc,compiler_builtins",
            "-Zbuild-std-features=compiler-builtins-mem",
        ])
        .status()
        .expect("Не удалось запустить сборку ядра");
    if !status.success() {
        panic!("Сборка ядра завершилась с ошибкой");
    }

    let release_dir = kernel_dir.join("target").join(kernel_target).join("release");
    let kernel_path = ["kernel", "kernel.exe"].iter()
        .map(|n| release_dir.join(n))
        .find(|p| p.exists())
        .unwrap_or_else(|| panic!("Не найден бинарник ядра в {}", release_dir.display()));

    eprintln!("Найдено ядро: {}", kernel_path.display());

    let bios_path = out_dir.join("bios.img");
    bootloader::BiosBoot::new(&kernel_path)
        .create_disk_image(&bios_path)
        .expect("Не удалось создать образ");

    println!("cargo:rustc-env=BIOS_PATH={}", bios_path.display());
    println!("cargo:rerun-if-changed=kernel");
    println!("cargo:rerun-if-changed=user");
}

fn build_user_programs() {
    let user_dir = PathBuf::from("user");
    let target = "x86_64-unknown-none";

    let status = Command::new("cargo")
        .current_dir(&user_dir)
        .args(["build", "--release", "--target", target, "-Zbuild-std=core"])
        .status()
        .expect("Не удалось собрать user-программы");
    if !status.success() {
        panic!("Сборка user-программ завершилась с ошибкой");
    }

    for name in ["user", "worker", "calculator"] {
        let base = user_dir.join("target").join(target).join("release");
        let a = base.join(name);
        let b = base.join(format!("{}.exe", name));
        let src = if a.exists() { a } else { b };
        let dst = PathBuf::from("kernel").join(format!("{}.elf", name));
        std::fs::copy(&src, &dst)
            .unwrap_or_else(|e| panic!("copy {} → {}: {}", src.display(), dst.display(), e));
        eprintln!("User ELF: {} → {}", src.display(), dst.display());
    }
}