#!/usr/bin/env python3
"""Slice the Paint toolbar atlas into one PNG per tool.

    python3 tools/paint_icons_split.py [atlas.png]

Reads assets/icons-src/paint-tools-atlas.png (72 x 24, 12 x 12 cells, 6 columns x 2 rows, in
the order of TOOLS in src/launcher/apps/paint.rs) and writes apps/paint/res/tools/<id>.png (Paint's
package: only Paint uses them).
Black stays ink, white is drawn in the cell's paper colour, transparent shows the plate through.
Cells with nothing in them are skipped, so the built-in glyph stays for that tool.
"""
import struct, sys, zlib
from pathlib import Path

IDS = ["t_pencil", "t_brush", "t_eraser", "t_hand", "t_line", "t_rect",
       "t_round", "t_ellipse", "t_fill", "t_spray", "t_text", "t_select"]
COLS, CELL = 6, 12
root = Path(__file__).resolve().parent.parent
src = Path(sys.argv[1]) if len(sys.argv) > 1 else root / "assets/icons-src/paint-tools-atlas.png"
out = root / "apps/paint/res/tools"


def chunks(d):
    p = 8
    while p < len(d):
        n, = struct.unpack(">I", d[p:p + 4])
        yield d[p + 4:p + 8], d[p + 8:p + 8 + n]
        p += 12 + n


def decode(path):
    idat, pal = b"", None
    for t, c in chunks(path.read_bytes()):
        if t == b"IHDR":
            w, h, bd, ct, _, _, il = struct.unpack(">IIBBBBB", c)
        elif t == b"PLTE":
            pal = c
        elif t == b"IDAT":
            idat += c
    assert bd == 8 and il == 0 and ct in (6, 3), "need an 8-bit RGBA (or palette) PNG without interlacing"
    ch = 4 if ct == 6 else 1
    raw, st, prev, o, rows = zlib.decompress(idat), w * ch, bytearray(w * ch), 0, []
    for _ in range(h):
        f, line = raw[o], bytearray(raw[o + 1:o + 1 + w * ch])
        o += 1 + st
        for i in range(st):
            a = line[i - ch] if i >= ch else 0
            b = prev[i]
            c_ = prev[i - ch] if i >= ch else 0
            if f == 1: line[i] = (line[i] + a) & 255
            elif f == 2: line[i] = (line[i] + b) & 255
            elif f == 3: line[i] = (line[i] + ((a + b) >> 1)) & 255
            elif f == 4:
                pa, pb, pc = abs(b - c_), abs(a - c_), abs(a + b - 2 * c_)
                line[i] = (line[i] + (a if pa <= pb and pa <= pc else (b if pb <= pc else c_))) & 255
        rows.append(line)
        prev = line
    px = [[(tuple(r[x * 4:x * 4 + 4]) if ct == 6 else (*pal[r[x] * 3:r[x] * 3 + 3], 255)) for x in range(w)] for r in rows]
    return w, h, px


def write(path, px):
    h, w = len(px), len(px[0])
    raw = b"".join(b"\x00" + b"".join(bytes(p) for p in row) for row in px)
    def ch(t, d):
        c = struct.pack(">I", len(d)) + t + d
        return c + struct.pack(">I", zlib.crc32(t + d) & 0xffffffff)
    path.write_bytes(b"\x89PNG\r\n\x1a\n" + ch(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
                     + ch(b"IDAT", zlib.compress(raw, 9)) + ch(b"IEND", b""))


w, h, px = decode(src)
assert (w, h) == (COLS * CELL, 2 * CELL), f"atlas must be {COLS * CELL} x {2 * CELL}, got {w} x {h}"
out.mkdir(parents=True, exist_ok=True)
for i, name in enumerate(IDS):
    x0, y0 = (i % COLS) * CELL, (i // COLS) * CELL
    cell = [[px[y0 + y][x0 + x] for x in range(CELL)] for y in range(CELL)]
    # Snap to the three tones the icon loader knows: black / white / transparent.
    cell = [[(0, 0, 0, 255) if p[3] >= 128 and (p[0] * 3 + p[1] * 6 + p[2]) < 1280 else
             (255, 255, 255, 255) if p[3] >= 128 else (0, 0, 0, 0) for p in row] for row in cell]
    f = out / f"{name}.png"
    if any(p[3] for row in cell for p in row):
        write(f, cell)
        print("wrote", f.relative_to(root))
    elif f.exists():
        f.unlink()
        print("removed (empty cell)", f.relative_to(root))
