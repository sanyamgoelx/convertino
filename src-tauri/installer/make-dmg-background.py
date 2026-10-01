#!/usr/bin/env python3
"""Background for the Mac .dmg window (660 x 400, plus a @2x copy).

Matches tauri.conf.json > bundle > macOS > dmg: Convertino at (170, 180),
Applications at (490, 180), 128 px icons. The CI joins the two PNGs into
dmg-background.tiff with `tiffutil -cathidpicheck` so Retina screens get
the sharp one.

Finder writes the icon names in black (light mode) or white (dark mode),
so each name sits on a mid-grey tag that both are readable on.
    python3 src-tauri/installer/make-dmg-background.py
"""
from pathlib import Path
from PIL import Image, ImageDraw, ImageFilter, ImageFont

HERE = Path(__file__).parent
W, H = 660, 400
APP, APPS, Y = 170, 490, 180
FONT = "/usr/share/fonts/opentype/noto/NotoSansCJK-Medium.ttc"

def draw(s):
    im = Image.new("RGB", (W * s, H * s), (0x14, 0x17, 0x1D))
    # soft blue glow behind the arrow
    glow = Image.new("L", im.size, 0)
    ImageDraw.Draw(glow).ellipse([200 * s, 80 * s, 460 * s, 300 * s], fill=70)
    glow = glow.filter(ImageFilter.GaussianBlur(60 * s))
    im.paste(Image.new("RGB", im.size, (0x0A, 0x3D, 0x6E)), (0, 0), glow)
    d = ImageDraw.Draw(im)
    f = ImageFont.truetype(FONT, 17 * s)
    t = "Drag Convertino into Applications"
    w = d.textlength(t, font=f)
    d.text(((W * s - w) / 2, 44 * s), t, font=f, fill=(0xF2, 0xF4, 0xF8))
    f2 = ImageFont.truetype(FONT, 12 * s)
    t2 = "Then open it from Applications"
    w2 = d.textlength(t2, font=f2)
    d.text(((W * s - w2) / 2, 72 * s), t2, font=f2, fill=(0x98, 0xA1, 0xB0))
    # arrow: three chevrons fading in
    for i, x in enumerate((296, 322, 348)):
        a = (0x2A + 0x22 * i, 0x5A + 0x18 * i, 0x9A + 0x18 * i)
        d.line([(x * s, (Y - 12) * s), ((x + 12) * s, Y * s), (x * s, (Y + 12) * s)],
               fill=a, width=4 * s, joint="curve")
    # name tags under both icons
    for cx in (APP, APPS):
        d.rounded_rectangle([(cx - 62) * s, (Y + 70) * s, (cx + 62) * s, (Y + 96) * s],
                            radius=13 * s, fill=(0x6B, 0x73, 0x85))
    return im

big = draw(2)
big.save(HERE / "dmg-background@2x.png")
big.resize((W, H), Image.LANCZOS).save(HERE / "dmg-background.png")
print("dmg-background.png, dmg-background@2x.png")
