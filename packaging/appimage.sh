#!/bin/sh
# Builds Fonyx-x86_64.AppImage: the desktop window, with the daemon and the
# CLI beside it, packed by Tauri's AppImage bundler. Opening it starts the
# bundled daemon if none is running and stops it again with the window (see
# BundledDaemon in ui/src-tauri/src/lib.rs).
#
# Needs the build dependencies from the README plus `cargo install tauri-cli
# --version '^2' --locked`. Build on the oldest distribution it should run
# on: the AppImage needs a glibc at least as new as the build machine's, and
# uses the system's own PipeWire (libpipewire, wpctl).
set -eu
cd "$(dirname "$0")/.."
cargo build --release --locked -p fonyx-daemon -p fonyx-cli
triple=$(rustc -vV | sed -n 's/^host: //p')
mkdir -p ui/src-tauri/binaries
for b in fonyx-daemon fonyx; do cp "target/release/$b" "ui/src-tauri/binaries/$b-$triple"; done
# The window is one file, ui/index.html; the bundler wants it in a folder of its
# own (tauri.conf.json serves ui/ itself, which also holds src-tauri).
mkdir -p target/appimage-web && cp ui/index.html target/appimage-web/
# The bundler and these paths only for this build: in tauri.conf.json, the
# sidecars would have to exist for every plain `cargo build`.
(cd ui/src-tauri && APPIMAGE_EXTRACT_AND_RUN=1 cargo tauri build --bundles appimage --config tauri.appimage.conf.json)
cp target/release/bundle/appimage/*.AppImage Fonyx-x86_64.AppImage
echo Fonyx-x86_64.AppImage
