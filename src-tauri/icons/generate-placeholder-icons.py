#!/usr/bin/env python3
"""Generate minimal, structurally valid placeholder app icons.

No external dependencies (no Pillow/ImageMagick required): PNG is
hand-encoded with stdlib `zlib`, ICO wraps PNG frames per the Vista+
ICO extension (any directory entry may hold a full PNG file instead of
a raw DIB), and ICNS wraps PNG frames in the modern PNG-backed OSType
chunks (`ic07`/`ic11`/`ic13`), the same scheme Apple's own `iconutil`
produces.

Every pixel is the same solid, opaque color -- there is no real
artwork here. This exists only so `tauri.conf.json`'s `bundle.icon`
list points at files that exist and decode cleanly, letting
`cargo check`/`cargo build`/`cargo test` succeed for `omnifrons-shell`
before the project has real app art (docs/repository-layout.md § Crate
map). Run it from this directory: `python3 generate-placeholder-icons.py`.
"""

from __future__ import annotations

import struct
import zlib
from pathlib import Path

ICON_DIR = Path(__file__).resolve().parent
# A flat, mid-tone slate placeholder color (opaque RGBA).
COLOR = (71, 85, 105, 255)


def encode_png(width: int, height: int, rgba: tuple[int, int, int, int]) -> bytes:
    """Hand-encode a solid-color RGBA PNG using only stdlib `zlib`."""

    def chunk(tag: bytes, data: bytes) -> bytes:
        return (
            struct.pack(">I", len(data))
            + tag
            + data
            + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
        )

    signature = b"\x89PNG\r\n\x1a\n"
    ihdr = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)  # 8-bit RGBA, no interlace
    row = b"\x00" + bytes(rgba) * width  # filter type 0 (None) per scanline
    raw = row * height
    idat = zlib.compress(raw, level=6)
    return signature + chunk(b"IHDR", ihdr) + chunk(b"IDAT", idat) + chunk(b"IEND", b"")


def build_ico(pngs: dict[int, bytes]) -> bytes:
    """Wrap PNG frames in an ICO container (PNG-in-ICO, Vista+)."""

    count = len(pngs)
    header = struct.pack("<HHH", 0, 1, count)  # reserved, type=icon, count
    entries = b""
    data = b""
    offset = 6 + 16 * count
    for size in sorted(pngs):
        png = pngs[size]
        # A directory dimension of 0 means 256 per the ICO spec.
        w = h = 0 if size == 256 else size
        entries += struct.pack("<BBBBHHII", w, h, 0, 0, 1, 32, len(png), offset)
        data += png
        offset += len(png)
    return header + entries + data


def build_icns(pngs: dict[str, bytes]) -> bytes:
    """Wrap PNG frames in an ICNS container using PNG-backed OSType chunks."""

    body = b""
    for tag, png in pngs.items():
        body += tag.encode("ascii") + struct.pack(">I", 8 + len(png)) + png
    total_len = 8 + len(body)
    return b"icns" + struct.pack(">I", total_len) + body


def main() -> None:
    png_32 = encode_png(32, 32, COLOR)
    png_128 = encode_png(128, 128, COLOR)
    png_256 = encode_png(256, 256, COLOR)

    (ICON_DIR / "32x32.png").write_bytes(png_32)
    (ICON_DIR / "128x128.png").write_bytes(png_128)
    (ICON_DIR / "128x128@2x.png").write_bytes(png_256)
    (ICON_DIR / "icon.png").write_bytes(png_128)

    ico = build_ico({32: png_32, 256: png_256})
    (ICON_DIR / "icon.ico").write_bytes(ico)

    icns = build_icns(
        {
            "ic11": png_32,  # 16x16@2x slot, holds a 32x32 PNG
            "ic07": png_128,  # 128x128
            "ic13": png_256,  # 128x128@2x slot, holds a 256x256 PNG
        }
    )
    (ICON_DIR / "icon.icns").write_bytes(icns)

    for f in sorted(ICON_DIR.glob("*")):
        if f.suffix in {".png", ".ico", ".icns"}:
            print(f"{f.name}: {f.stat().st_size} bytes")


if __name__ == "__main__":
    main()
