#!/usr/bin/env python3
"""Render 12x12 bitmaps for CJK chars missing from JIS X 0208, from Noto Sans SC.

Source: NotoSansSC-Regular.ttf (OFL) already in the workspace, used by the
CardputerZero LVGL tree. Output: BDF STARTCHAR blocks using the empty
JIS X 0208 ku 85 (0x7521..) as gaiji slots, plus a text preview.
"""
from PIL import Image, ImageDraw, ImageFont

TTF = "/home/hatch/workspace/cardputer-zero-muse/build-launcher/SDK/github_source/lvgl/lvgl_9_5/lvgl/tests/src/test_files/fonts/noto/NotoSansSC-Regular.ttf"
# (char, JIS kuten code as hex string, ENCODING decimal)
CHARS = [
    ("吧", "7521"),
    ("呢", "7522"),
    ("嗎", "7523"),
    ("嘅", "7524"),
    ("咗", "7525"),
    ("冇", "7526"),
    ("啲", "7527"),
    ("佢", "7528"),
]

def render_12(ch: str):
    font = ImageFont.truetype(TTF, 12)
    # Render big-ish, then take the ink bounding box and centre it in 12x12.
    img = Image.new("L", (24, 24), 0)
    d = ImageDraw.Draw(img)
    d.text((6, 6), ch, font=font, fill=255)
    bbox = img.getbbox()
    assert bbox, f"no ink for {ch}"
    x0, y0, x1, y1 = bbox
    w, h = x1 - x0, y1 - y0
    assert w <= 12 and h <= 12, f"{ch} bbox {w}x{h} exceeds 12x12"
    crop = img.crop(bbox)
    out = Image.new("L", (12, 12), 0)
    out.paste(crop, ((12 - w) // 2, (12 - h) // 2))
    bits = []
    for y in range(12):
        row = 0
        for x in range(12):
            if out.getpixel((x, y)) > 100:
                row |= 1 << (11 - x)
        bits.append(row)
    return bits

def main():
    blocks = []
    for ch, jis_hex in CHARS:
        bits = render_12(ch)
        assert any(bits), f"empty glyph {ch}"
        print(f"U+{ord(ch):04X} {ch} -> JIS 0x{jis_hex} (ENCODING {int(jis_hex, 16)})")
        for row in bits:
            print("".join("#" if row & (1 << (11 - x)) else "." for x in range(12)))
        print()
        rows = "\n".join(f"{b << 4:04X}" for b in bits)
        blocks.append(
            f"STARTCHAR {jis_hex}\n"
            f"ENCODING {int(jis_hex, 16)}\n"
            f"SWIDTH 960 0\nDWIDTH 12 0\nBBX 12 12 0 -2\nBITMAP\n{rows}\nENDCHAR"
        )
    with open("/tmp/gaiji_blocks.bdf", "w") as f:
        f.write("\n".join(blocks) + "\n")
    print("wrote /tmp/gaiji_blocks.bdf")

if __name__ == "__main__":
    main()
