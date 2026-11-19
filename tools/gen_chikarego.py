#!/usr/bin/env python3
"""Convert ChiKareGo2.ttf (Giles Booth, CC BY; a 16 px BitFontMaker2 grid, 64 units per pixel) into a
Playdate-style .fnt that PlaydateFont reads (a base64 PNG sheet, one advance per glyph).

The cell is 12 rows tall with the capital top at row 0 (so it drops into the places Cavs used: caps rows
0..8, baseline row 9, descenders to row 11). Anything above the cap top (accents on capitals) is cut.

usage: gen_chikarego.py ChiKareGo2.ttf assets/fonts/ChiKareGo2.fnt
"""
import base64, io, sys
from fontTools.ttLib import TTFont
from PIL import Image, ImageDraw, ImageFont

ttf, out = sys.argv[1], sys.argv[2]
tt = TTFont(ttf)
cmap = tt.getBestCmap()
hmtx = tt["hmtx"]
UPEM_PX = tt["head"].unitsPerEm // 64          # 16
ASC = tt["hhea"].ascent // 64                  # 12 rows above the baseline
CAP = 9                                        # capital height in px (576 / 64)
font = ImageFont.truetype(ttf, UPEM_PX)
CELL_H = 12
TOP = ASC - CAP                                # image rows above the cap top

chars = [cp for cp in sorted(cmap) if 32 <= cp < 256 and cp != 0x7F]
glyphs = []
for cp in chars:
    ch = chr(cp)
    adv = hmtx[cmap[cp]][0] // 64
    im = Image.new("1", (24, 20), 0)
    d = ImageDraw.Draw(im)
    d.fontmode = "1"
    d.text((0, 0), ch, font=font, fill=1)
    cell = im.crop((0, TOP, 24, TOP + CELL_H))
    bbox = cell.getbbox()
    glyphs.append((ch, adv, cell, bbox))

CELL_W = max([g[3][2] for g in glyphs if g[3]] + [8])
cols = 16
rows = (len(glyphs) + cols - 1) // cols
sheet = Image.new("RGBA", (cols * CELL_W, rows * CELL_H), (0, 0, 0, 0))
px = sheet.load()
for i, (ch, adv, cell, bbox) in enumerate(glyphs):
    ox, oy = (i % cols) * CELL_W, (i // cols) * CELL_H
    cp = cell.load()
    for y in range(CELL_H):
        for x in range(min(24, CELL_W)):
            if cp[x, y]:
                px[ox + x, oy + y] = (0, 0, 0, 255)
buf = io.BytesIO()
sheet.save(buf, "PNG", optimize=True)
data = base64.b64encode(buf.getvalue()).decode()
lines = [
    "--metrics={\"baseline\":-3,\"xHeight\":6,\"capHeight\":0}",
    f"datalen={len(buf.getvalue())}",
    f"data={data}",
    f"width={CELL_W}",
    f"height={CELL_H}",
    "",
    "tracking=-1",
    "",
]
for ch, adv, cell, bbox in glyphs:
    key = "space" if ch == " " else ch
    if ch == "\t":
        continue
    lines.append(f"{key}\t{adv}")
open(out, "w").write("\n".join(lines) + "\n")
print(f"{len(glyphs)} glyphs, cell {CELL_W}x{CELL_H}, sheet {sheet.size}, wrote {out}")
