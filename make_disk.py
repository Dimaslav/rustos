#!/usr/bin/env python3
"""
make_disk.py — собирает fat32.img:
  MBR (512 байт) | padding до 1 MiB | FAT32-партиция

Содержимое партиции берётся из assets/.
Запуск: python3 make_disk.py [--force]
"""

import os
import shutil
import struct
import subprocess
import sys

IMG = "fat32.img"
PART_IMG = "part.img"
SIZE_MB = 64              # общий размер образа
PART_START_LBA = 2048     # 1 MiB — классическое выравнивание
SECTOR = 512

# ---------------- утилиты ----------------

def die(msg: str) -> None:
    print(f"error: {msg}", file=sys.stderr)
    sys.exit(1)

def require_tool(name: str) -> None:
    if shutil.which(name) is None:
        die(
            f"'{name}' не найден. Установите:\n"
            f"  Debian/Ubuntu: sudo apt install mtools dosfstools\n"
            f"  Arch:          sudo pacman -S mtools dosfstools\n"
            f"  macOS:         brew install mtools dosfstools"
        )

# ---------------- генерация обоев ----------------

def make_wallpaper(path: str) -> None:
    """Простые градиентные обои 1920x1080 в BMP 24-bit."""
    W, H = 1920, 1080
    row_size = (W * 3 + 3) & ~3
    pad = row_size - W * 3
    pix_size = row_size * H
    file_size = 54 + pix_size

    with open(path, "wb") as f:
        # BITMAPFILEHEADER
        f.write(b"BM")
        f.write(struct.pack("<I", file_size))
        f.write(struct.pack("<HH", 0, 0))
        f.write(struct.pack("<I", 54))
        # BITMAPINFOHEADER
        f.write(struct.pack("<I", 40))
        f.write(struct.pack("<i", W))
        f.write(struct.pack("<i", H))
        f.write(struct.pack("<H", 1))
        f.write(struct.pack("<H", 24))
        f.write(struct.pack("<I", 0))
        f.write(struct.pack("<I", pix_size))
        f.write(struct.pack("<i", 2835))
        f.write(struct.pack("<i", 2835))
        f.write(struct.pack("<I", 0))
        f.write(struct.pack("<I", 0))
        # пиксели: BGR, bottom-up
        for y in range(H):
            row = bytearray()
            for x in range(W):
                t = (x / W + y / H) / 2.0
                r = int(20 + t * 50)
                g = int(30 + t * 25)
                b = int(70 + t * 70)
                row += bytes((b, g, r))
            row += b"\x00" * pad
            f.write(row)

# ---------------- MBR ----------------

def make_mbr(part_start_lba: int, part_sectors: int) -> bytes:
    mbr = bytearray(512)
    e = 446  # первый partition entry
    mbr[e + 0] = 0x80                              # bootable
    # CHS start (bytes 1..3) — оставляем нулями, ядро использует LBA
    mbr[e + 4] = 0x0C                              # тип: FAT32 LBA
    # CHS end (bytes 5..7) — нули
    mbr[e + 8:e + 12] = struct.pack("<I", part_start_lba)
    mbr[e + 12:e + 16] = struct.pack("<I", part_sectors)
    mbr[510] = 0x55
    mbr[511] = 0xAA
    return bytes(mbr)

# ---------------- сборка ----------------

def build_partition() -> int:
    """Создаёт и форматирует part.img. Возвращает размер в секторах."""
    part_mb = SIZE_MB - (PART_START_LBA * SECTOR) // (1024 * 1024)
    print(f"[make_disk] создаю партицию {part_mb} MiB")

    with open(PART_IMG, "wb") as f:
        f.truncate(part_mb * 1024 * 1024)

    print("[make_disk] mkfs.fat -F 32")
    subprocess.run(
        ["mkfs.fat", "-F", "32", "-n", "RUSTOS", PART_IMG],
        check=True,
        stdout=subprocess.DEVNULL,
    )

    # Копируем содержимое assets/
    if os.path.isdir("assets"):
        files = sorted(
            os.path.join("assets", n)
            for n in os.listdir("assets")
            if os.path.isfile(os.path.join("assets", n))
        )
        for path in files:
            print(f"[make_disk]   + {os.path.basename(path)}")
            subprocess.run(["mcopy", "-i", PART_IMG, path, "::"], check=True)
    else:
        print("[make_disk] assets/ не найден — партиция будет пустой")

    return part_mb * 1024 * 1024 // SECTOR

def main() -> None:
    force = "--force" in sys.argv
    if os.path.exists(IMG) and not force:
        print(f"[make_disk] {IMG} уже существует. Используйте --force.")
        return

    require_tool("mkfs.fat")
    require_tool("mcopy")

    # Обои
    os.makedirs("assets", exist_ok=True)
    wp = os.path.join("assets", "WALLPAPER.BMP")
    if not os.path.exists(wp):
        print("[make_disk] генерирую assets/WALLPAPER.BMP")
        make_wallpaper(wp)

    part_sectors = build_partition()

    print(f"[make_disk] собираю {IMG} = MBR + padding + партиция")
    with open(IMG, "wb") as out:
        out.write(make_mbr(PART_START_LBA, part_sectors))
        out.write(b"\x00" * (PART_START_LBA * SECTOR - 512))
        with open(PART_IMG, "rb") as p:
            shutil.copyfileobj(p, out)

    os.remove(PART_IMG)
    total = os.path.getsize(IMG) // (1024 * 1024)
    print(f"[make_disk] готово: {IMG} ({total} MiB)")

if __name__ == "__main__":
    main()