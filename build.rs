use std::path::{Path, PathBuf};
use std::process::Command;

const USER_BINS: [&str; 8] = ["user", "worker", "calculator", "shell", "echo", "ls", "cat", "ps"];

fn main() {
    build_user_programs();

    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let kernel_dir = PathBuf::from("kernel");
    let kernel_target = "x86_64-unknown-none";

    let headless = std::env::var("CARGO_FEATURE_HEADLESS_TEST").is_ok();
    let mut kernel_args: Vec<&str> = vec![
        "build", "--release", "--target", kernel_target,
        "-Zbuild-std=core,alloc,compiler_builtins",
        "-Zbuild-std-features=compiler-builtins-mem",
    ];
    if headless {
        kernel_args.push("--features");
        kernel_args.push("headless_test");
    }

    let status = Command::new("cargo")
        .current_dir(&kernel_dir)
        .args(&kernel_args)
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
    println!("cargo:rerun-if-changed=user/linker.ld");
    println!("cargo:rerun-if-changed=user/.cargo/config.toml");
    for name in USER_BINS.iter() {
        println!("cargo:rerun-if-changed=kernel/{}.elf", name);
    }
}

fn build_user_programs() {
    let user_dir = PathBuf::from("user");
    let target = "x86_64-unknown-none";

    let status = Command::new("cargo")
        .current_dir(&user_dir)
        .args([
            "build",
            "--release",
            "--target", target,
            "-Zbuild-std=core,alloc,compiler_builtins",
            "-Zbuild-std-features=compiler-builtins-mem",
        ])
        .status()
        .expect("Не удалось собрать user-программы");
    if !status.success() {
        panic!("Сборка user-программ завершилась с ошибкой");
    }

    for name in USER_BINS.iter() {
        let base = user_dir.join("target").join(target).join("release");
        let a = base.join(name);
        let b = base.join(format!("{}.exe", name));
        let src = if a.exists() { a } else { b };

        let dst = PathBuf::from("kernel").join(format!("{}.elf", name));
        std::fs::copy(&src, &dst)
            .unwrap_or_else(|e| panic!("copy {} → {}: {}", src.display(), dst.display(), e));

        fixup_elf_base(&dst, 0x40_0000)
            .unwrap_or_else(|e| panic!("fixup {}: {}", dst.display(), e));

        let bytes = std::fs::read(&dst).unwrap();
        let e_type = u16::from_le_bytes([bytes[16], bytes[17]]);
        let e_entry = u64::from_le_bytes(bytes[24..32].try_into().unwrap());
        eprintln!(
            "[build] {} size={} type={} e_entry={:#x}",
            dst.display(), bytes.len(), e_type, e_entry
        );

        if e_type != 2 || e_entry < 0x40_0000 {
            panic!(
                "после fixup ожидали ET_EXEC с entry ≥ 0x400000, получили type={} entry={:#x}",
                e_type, e_entry
            );
        }
    }
}

fn fixup_elf_base(path: &Path, base: u64) -> Result<(), String> {
    let mut bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if bytes.len() < 64 || &bytes[0..4] != b"\x7FELF" {
        return Err("not ELF".into());
    }
    if bytes[4] != 2 {
        return Err("not ELF64".into());
    }

    let e_type = u16::from_le_bytes([bytes[16], bytes[17]]);
    let e_entry_before = u64::from_le_bytes(bytes[24..32].try_into().unwrap());

    if e_type == 2 && e_entry_before >= base {
        return Ok(());
    }
    if e_type != 2 && e_type != 3 {
        return Err(format!("unsupported e_type={}", e_type));
    }

    bytes[16] = 2;
    bytes[17] = 0;
    bytes[24..32].copy_from_slice(&(e_entry_before + base).to_le_bytes());

    let e_phoff = u64::from_le_bytes(bytes[32..40].try_into().unwrap()) as usize;
    let e_phentsize = u16::from_le_bytes([bytes[54], bytes[55]]) as usize;
    let e_phnum = u16::from_le_bytes([bytes[56], bytes[57]]) as usize;

    if e_phoff + e_phnum * e_phentsize > bytes.len() {
        return Err("phdr за пределами файла".into());
    }

    for i in 0..e_phnum {
        let p = e_phoff + i * e_phentsize;
        let p_type = u32::from_le_bytes(bytes[p..p + 4].try_into().unwrap());
        if p_type != 1 { continue; }
        let v = u64::from_le_bytes(bytes[p + 16..p + 24].try_into().unwrap());
        bytes[p + 16..p + 24].copy_from_slice(&(v + base).to_le_bytes());
        let a = u64::from_le_bytes(bytes[p + 24..p + 32].try_into().unwrap());
        bytes[p + 24..p + 32].copy_from_slice(&(a + base).to_le_bytes());
    }

    std::fs::write(path, &bytes).map_err(|e| e.to_string())?;
    Ok(())
}