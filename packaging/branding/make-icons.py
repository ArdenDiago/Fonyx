#!/usr/bin/env python3
"""Cut the icons out of the logo artwork. Run from the repository root:

    python3 packaging/branding/make-icons.py

AppLogo.jpg is the artwork as drawn: the plate on a cloth background. The
icons are the plate alone, with everything outside its rounded corners made
transparent. Needs Pillow.
"""
from PIL import Image, ImageDraw

# Where the plate sits in AppLogo.jpg. Change these if the artwork changes.
LEFT, TOP, SIZE = 274, 48, 476

plate = Image.open('packaging/branding/AppLogo.jpg').convert('RGB').crop((LEFT, TOP, LEFT + SIZE, TOP + SIZE))
big = plate.resize((SIZE * 4, SIZE * 4), Image.LANCZOS)  # 4x, so the corners come out smooth
mask = Image.new('L', big.size, 0)
ImageDraw.Draw(mask).rounded_rectangle((6, 6, SIZE * 4 - 7, SIZE * 4 - 10), radius=118, fill=255)
big.putalpha(mask)

for path, px in (
    ('ui/src-tauri/icons/icon.png', 512),  # the app: window, launcher and task bar
    ('site/public/logo.png', 192),         # the website's top bar
    ('site/public/favicon.png', 64),       # the browser tab
):
    big.resize((px, px), Image.LANCZOS).save(path, optimize=True)
    print(path, px)
