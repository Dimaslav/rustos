# Rust OS

## Сборка и запуск
    cargo run

## Smoke-тесты
    cargo run --features headless_test -- --test

## Отладка

1. Установи GDB. Проще всего WinLibs (portable, без админа):
   - https://winlibs.com/ → "GCC + GDB + LLVM/Clang (without IDE)", UCRT runtime
   - Распакуй в `C:\winlibs\`
   - Добавь `C:\winlibs\mingw64\bin` в PATH через GUI
     (`Win+R` → `sysdm.cpl` → Переменные среды → Path → Создать)
   - Закрой все cmd/PowerShell, открой новое, проверь: `gdb --version`

2. Терминал 1: `powershell -ExecutionPolicy Bypass -File debug.ps1`

3. Терминал 2: `gdb -x .gdbinit`

В GDB: `c` — продолжить, `Ctrl+C` — остановить, `bt` — стек,
`p/x $cr2` — адрес page fault, `pte 0x400000` — разбор virt-адреса.

## Структура
    kernel/    — ядро
    user/      — Ring 3 программы
    src/main.rs — launcher QEMU
    build.rs    — сборка + bios.img
    make_disk.py — fat32.img