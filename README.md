# Rust OS

Учебная операционная система на Rust для x86_64.
Монолитное ядро, Ring 3, FAT32, GUI, планировщик, syscalls.

## Требования

- **Rust nightly** — установлен через [rustup](https://rustup.rs).
- **QEMU** — [qemu.weilnetz.de/w64](https://qemu.weilnetz.de/w64/) (Windows).
  При установке поставь галочку «Add QEMU to the system PATH».
- **Python 3** — [python.org](https://www.python.org/downloads/) (для `make_disk.py`).
  При установке — галочка «Add python.exe to PATH».
- **GDB** — для отладки ядра. На Windows не идёт с rustup, ставь отдельно (см. ниже).

Проверка:
```cmd
cargo --version
qemu-system-x86_64 --version
python --version
gdb --version