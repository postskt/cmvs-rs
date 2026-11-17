#!/usr/bin/env python3
"""Patch a nacptool-generated control.nacp: real (UTF-8) title/author in all 16
language slots, plus the hardware-proven golden-reference field values
(see NSP-PACKING.md §3).

Why not pass the title to nacptool directly: devkitPro tools may read argv as
ANSI, so a CJK title gets mangled. nacptool writes an ASCII placeholder; this
script (Unicode-safe) overwrites the slots afterwards.

Fields written (switchbrew NACP):
  16 x 0x300 language slots: name (0x200) + author (0x100), UTF-8, NUL padded
  0x3025  startup_user_account = 1 (Required)
  0x302C  supported-language bitmask = 0x0BFF
  0x3060  display version, printable ASCII, no leading space
  0x3080  UserAccountSaveDataSize       = 0x3E00000 (62 MB, nacptool default)
  0x3088  UserAccountSaveDataJournalSize = 0x180000  (1.5 MB)
  0x3090  DeviceSaveDataSize            = 0

Usage:
  python patch_nacp.py <control.nacp> --title "游戏名" --author "作者" [--version 1.0.0]
"""
from __future__ import annotations

import argparse
import struct
import sys
from pathlib import Path

NACP_SIZE = 0x4000
SLOT = 0x300
NAME_LEN = 0x200
AUTHOR_LEN = 0x100


def put_text(data: bytearray, off: int, length: int, text: str, what: str) -> None:
    raw = text.encode("utf-8")
    if len(raw) >= length:
        raise SystemExit(f"{what} is too long for NACP ({len(raw)} bytes, max {length - 1}): {text!r}")
    data[off:off + length] = raw.ljust(length, b"\0")


def patch(data: bytearray, title: str, author: str, version: str) -> None:
    if len(data) != NACP_SIZE:
        raise SystemExit(f"control.nacp is {len(data)} bytes, expected {NACP_SIZE}")
    version = version.strip()
    if not version or not all(32 <= ord(c) < 127 for c in version):
        raise SystemExit(f"display version must be printable ASCII without leading space: {version!r}")
    for i in range(16):
        put_text(data, i * SLOT, NAME_LEN, title, f"title slot {i}")
        put_text(data, i * SLOT + NAME_LEN, AUTHOR_LEN, author, f"author slot {i}")
    data[0x3025] = 1
    struct.pack_into("<I", data, 0x302C, 0x0BFF)
    put_text(data, 0x3060, 0x10, version, "display version")
    struct.pack_into("<Q", data, 0x3080, 0x3E00000)
    struct.pack_into("<Q", data, 0x3088, 0x180000)
    struct.pack_into("<Q", data, 0x3090, 0)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("nacp")
    ap.add_argument("--title", required=True)
    ap.add_argument("--author", required=True)
    ap.add_argument("--version", default="1.0.0")
    ap.add_argument("--titleid", default=None,
                    help="verify nacptool --titleid wrote this id at 0x3038 (hacBrewPack reads it)")
    a = ap.parse_args()
    p = Path(a.nacp)
    data = bytearray(p.read_bytes())
    if a.titleid:
        want = int(a.titleid, 16)
        got = struct.unpack_from("<Q", data, 0x3038)[0]
        if got != want:
            raise SystemExit(f"NACP 0x3038 title id is 0x{got:016X}, expected 0x{want:016X}; "
                             "run nacptool with --titleid=")
    patch(data, a.title, a.author, a.version)
    p.write_bytes(data)
    print(f"nacp patched: title={a.title!r} author={a.author!r} version={a.version!r}")
    print("  16 UTF-8 language slots, startup_user_account=1, languages=0x0BFF,")
    print("  save sizes 0x3E00000/0x180000, device save 0 (golden reference values)")
    return 0


if __name__ == "__main__":
    sys.exit(main())