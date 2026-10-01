#!/usr/bin/env python3
"""Logo bitmaps for the Windows installer window (ui.nsh).

Plain 24-bit BMPs with the window colour already behind the icon, because
the installer's picture control has no transparency. One per display scale.
Run from the repo root after changing assets/app-icon.png:
    python3 src-tauri/installer/make-images.py
"""
from pathlib import Path
from PIL import Image

HERE = Path(__file__).parent
BG = (0x14, 0x17, 0x1D)  # CV_BG in ui.nsh
LOGICAL = 88             # logo size at 100 %

icon = Image.open(HERE.parent.parent / "assets" / "app-icon.png").convert("RGBA")
for scale in (100, 125, 150, 175, 200, 250, 300):
    px = round(LOGICAL * scale / 100)
    im = Image.new("RGBA", (px, px), BG + (255,))
    im.alpha_composite(icon.resize((px, px), Image.LANCZOS))
    im.convert("RGB").save(HERE / f"logo-{scale}.bmp")
    print(f"logo-{scale}.bmp {px}px")
