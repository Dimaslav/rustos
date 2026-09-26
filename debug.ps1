# debug.ps1 — QEMU с gdb-stub, ждёт подключения GDB.
# В другом терминале: gdb -x .gdbinit

param([switch]$Log)

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

$biosImg = Get-ChildItem -Recurse -Filter bios.img -Path target -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $biosImg) {
    Write-Host "[debug] bios.img не найден — запускаю cargo build"
    & cargo build
    if ($LASTEXITCODE -ne 0) { exit 1 }
    $biosImg = Get-ChildItem -Recurse -Filter bios.img -Path target -ErrorAction SilentlyContinue | Select-Object -First 1
}
if (-not $biosImg) { Write-Host "[debug] ERROR: bios.img не найден"; exit 1 }

$diskArgs = @()
if (Test-Path "fat32.img") {
    $diskArgs = @("-drive", "format=raw,file=fat32.img,if=ide,index=1,media=disk")
}

$qemuArgs = @(
    "-drive", "format=raw,file=$($biosImg.FullName)"
) + $diskArgs + @(
    "-serial", "stdio",
    "-S", "-s",
    "-no-reboot",
    "-no-shutdown",
    "-display", "gtk"
)

if ($Log) {
    $qemuArgs += @("-d", "int,cpu_reset", "-D", "qemu.log")
    Write-Host "[debug] -Log: qemu.log будет записан"
}

Write-Host ""
Write-Host "=========================================================="
Write-Host " QEMU запущен с замороженным CPU. Serial — в этом окне."
Write-Host ""
Write-Host " В ДРУГОМ терминале:  gdb -x .gdbinit"
Write-Host " GDB подключится и продолжит выполнение."
Write-Host " Управление:  c=continue  bt=backtrace  Ctrl+C=стоп"
Write-Host "=========================================================="
Write-Host ""

& qemu-system-x86_64 @qemuArgs