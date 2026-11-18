#!/usr/bin/env python3
"""Draw the app icon and the three tray template icons. No dependencies."""
import struct, sys, zlib
from pathlib import Path

def png(path, w, h, px):
    raw = b"".join(b"\x00" + bytes(c for x in range(w) for c in px(x, y)) for y in range(h))
    def chunk(tag, data):
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data))
    Path(path).write_bytes(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
                           + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b""))

def rrect(x, y, x0, y0, x1, y1, r):
    """Signed distance-ish coverage (0..1) of a rounded rectangle, antialiased."""
    cx = min(max(x, x0 + r), x1 - r); cy = min(max(y, y0 + r), y1 - r)
    d = ((x - cx) ** 2 + (y - cy) ** 2) ** 0.5 - r
    if x0 + r <= x <= x1 - r or y0 + r <= y <= y1 - r:
        d = max(x0 - x, x - x1, y0 - y, y - y1)
    return min(max(0.5 - d, 0.0), 1.0)

def glyph(x, y, s, variant):
    """Clipboard glyph coverage at unit scale s (glyph designed on a 22pt grid)."""
    u = lambda v: v * s
    board = rrect(x, y, u(4), u(3.5), u(18), u(20.5), u(2.6))
    inner = rrect(x, y, u(5.6), u(5.1), u(16.4), u(18.9), u(1.3))
    clip = rrect(x, y, u(7.5), u(1.5), u(14.5), u(6), u(1.5))
    a = max(board - inner, clip)
    if variant == "normal":
        for ly in (9.5, 12.5, 15.5):
            a = max(a, rrect(x, y, u(7.6), u(ly - .7), u(14.4 if ly < 15 else 12), u(ly + .7), u(.7)))
    elif variant == "paused":
        a = max(a, rrect(x, y, u(8), u(9), u(10.2), u(16.5), u(.6)), rrect(x, y, u(11.8), u(9), u(14), u(16.5), u(.6)))
    elif variant == "blocked":
        a = max(a, rrect(x, y, u(10.1), u(8.2), u(11.9), u(14), u(.9)), rrect(x, y, u(10.1), u(15.3), u(11.9), u(17.1), u(.9)))
    return a

out = Path(sys.argv[1] if len(sys.argv) > 1 else "app/icons")
for variant in ("normal", "paused", "blocked"):
    png(out / f"tray-{variant}.png", 44, 44,
        lambda x, y: (0, 0, 0, int(255 * glyph(x + .5, y + .5, 2, variant) * (1 if variant == "normal" else .9))))

N = 1024
def app_px(x, y):
    x += .5; y += .5
    bg = rrect(x, y, 100, 100, 924, 924, 185)
    t = y / N
    r, g, b = (int(a + (c - a) * t) for a, c in ((72, 34), (118, 60), (255, 190)))
    g2 = glyph(x - 232, y - 232, 560 / 22, "normal")
    r, g, b = (int(v + (255 - v) * g2) for v in (r, g, b))
    return (r, g, b, int(255 * bg))
png(out / "app-icon.png", N, N, app_px)
print("icons written to", out)
