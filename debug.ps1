# debug.ps1 — запускает QEMU с gdb-stub для отладки ядра.
#
# QEMU стартует с замороженным CPU (-S) и слушает GDB на localhost:1234 (-s).
# В другом терминале запусти:
#     gdb -x .gdbinit
#
# После подключения GDB выполнит `c` (continue), и ядро начнёт работать.
#
# Параметры:
#   -Log        — писать qemu.log с трассировкой прерываний (очень многословно,
#                 но полезно при тройных фолтах).
#   -Release    — использовать release-сборку (по умолчанию build.rs делает её).
#   -DebugBuild — использовать debug-сборку kernel (медленнее, но без оптимизаций).

param(
    [switch]$Log,
    [switch]$DebugBuild
)

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

# ---------- Собрать проект, если нужно ----------

$biosImg = Get-ChildItem -Recurse -Filter bios.img -Path target -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $biosImg) {
    Write-Host "[debug] bios.img не найден — запускаю cargo build"
    & cargo build
    if ($LASTEXITCODE -ne 0) { exit 1 }
    $biosImg = Get-ChildItem -Recurse -Filter bios.img -Path target -ErrorAction SilentlyContinue | Select-Object -First 1
}
if (-not $biosImg) {
    Write-Host "[debug] ERROR: bios.img не найден даже после сборки"
    exit 1
}

Write-Host "[debug] BIOS: $($biosImg.FullName)"

# ---------- Диск (если есть) ----------

$diskArgs = @()
if (Test-Path "fat32.img") {
    $diskArgs = @("-drive", "format=raw,file=fat32.img,if=ide,index=1,media=disk")
}

# ---------- QEMU ----------

$qemuArgs = @(
    "-drive", "format=raw,file=$($biosImg.FullName)"
) + $diskArgs + @(
    "-serial", "stdio",
    "-S", "-s",                # -S: заморозить CPU; -s: gdb-stub на :1234
    "-no-reboot",
    "-no-shutdown",
    "-display", "gtk"
)

if ($Log) {
    Write-Host "[debug] -Log включён: пишу qemu.log (трассировка прерываний)"
    $qemuArgs += @("-d", "int,cpu_reset", "-D", "qemu.log")
}

Write-Host ""
Write-Host "======================================================================="
Write-Host " QEMU запущен с замороженным CPU. Serial — в этом окне."
Write-Host ""
Write-Host " В ДРУГОМ терминале выполни:"
Write-Host "     gdb -x .gdbinit"
Write-Host ""
Write-Host " GDB подключится к localhost:1234 и сразу продолжит выполнение."
Write-Host " Управление в GDB:"
Write-Host "     c            продолжить"
Write-Host "     bt           backtrace"
Write-Host "     Ctrl+C       остановить и вернуться в GDB"
Write-Host ""
Write-Host " Чтобы прервать QEMU: Ctrl+C здесь."
Write-Host "======================================================================="
Write-Host ""

& qemu-system-x86_64 @qemuArgs