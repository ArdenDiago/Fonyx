#!/usr/bin/env python3
"""Cut the icons from the logo. Run from the repository root:

    python3 packaging/branding/make-icons.py

logo.svg is the artwork: the mark on its rounded square, transparent outside
the corners. Needs rsvg-convert (librsvg).
"""
import subprocess

for path, px in (
    ('ui/src-tauri/icons/icon.png', 512),  # the app: window, launcher and task bar
    ('site/public/logo.png', 192),         # the website's top bar
    ('site/public/favicon.png', 64),       # the browser tab
):
    subprocess.run(['rsvg-convert', '-w', str(px), '-h', str(px), 'packaging/branding/logo.svg', '-o', path], check=True)
    print(path, px)
