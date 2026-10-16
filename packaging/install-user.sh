#!/bin/sh
# Installs the daemon as a systemd *user* service without root: starts at
# login, restarts on crash or hang (see the unit's watchdog). Re-run after
# pulling changes to install the new build.
set -eu
cd "$(dirname "$0")/.."

cargo build --release -p fonyx-daemon -p fonyx-ui -p fonyx-cli

# Copy then rename: overwriting a running binary in place fails (ETXTBSY).
mkdir -p "$HOME/.local/bin"
install -m755 target/release/fonyx-daemon "$HOME/.local/bin/fonyx-daemon.new"
mv -f "$HOME/.local/bin/fonyx-daemon.new" "$HOME/.local/bin/fonyx-daemon"
install -m755 target/release/fonyx-ui "$HOME/.local/bin/fonyx-ui.new"
mv -f "$HOME/.local/bin/fonyx-ui.new" "$HOME/.local/bin/fonyx-ui"
# The CLI (`fonyx --help`).
install -m755 target/release/fonyx "$HOME/.local/bin/fonyx.new"
mv -f "$HOME/.local/bin/fonyx.new" "$HOME/.local/bin/fonyx"

# Launcher entry and icon, so the app shows up in menus and task bars with its
# logo. The entry is named after the app's window id (tauri.conf.json's
# identifier), which is how a desktop matches a window to its icon.
data="${XDG_DATA_HOME:-$HOME/.local/share}"
install -Dm644 ui/src-tauri/icons/icon.png "$data/icons/hicolor/512x512/apps/org.fonyx.App.png"
mkdir -p "$data/applications"
sed "s|^Exec=fonyx-ui|Exec=$HOME/.local/bin/fonyx-ui|" \
  packaging/flatpak/org.fonyx.App.desktop >"$data/applications/org.fonyx.app.desktop"

unit_dir="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
mkdir -p "$unit_dir"

# Retire an install made under one of the project's old names, so two daemons
# never compete for the audio graph. Its config is moved over by the new daemon.
for name in sonicforge mtxpanel; do
  for old in "$name-ui" "$name-quickshell" "$name-daemon"; do
    systemctl --user disable --now "$old.service" 2>/dev/null || true
    rm -f "$unit_dir/$old.service" "$HOME/.local/bin/$old"
  done
  pkill -x "$name-daemon" 2>/dev/null || true
  rm -rf "$data/$name"
done
rm -f "$HOME/.local/bin/sonicforge" "$data/applications/org.sonicforge.app.desktop" \
  "$data/icons/hicolor/512x512/apps/org.sonicforge.App.png"
sed 's|^ExecStart=/usr/bin/fonyx-daemon|ExecStart=%h/.local/bin/fonyx-daemon|' \
  packaging/systemd/fonyx-daemon.service >"$unit_dir/fonyx-daemon.service"
# The Tauri UI is what opens at login.
sed 's|^ExecStart=/usr/bin/fonyx-ui|ExecStart=%h/.local/bin/fonyx-ui|' \
  packaging/systemd/fonyx-ui.service >"$unit_dir/fonyx-ui.service"

# Quickshell files + its unit (the packaged unit expects /usr/share/fonyx).
# Not enabled here (Settings tab toggle). FonyxPopup.qml is the fader
# popup meant for a compositor keybind, e.g. niri:
#   Mod+A { spawn-sh "pkill -f FonyxPopup.qml || quickshell -p ~/.local/share/fonyx/quickshell/FonyxPopup.qml"; }
share="${XDG_DATA_HOME:-$HOME/.local/share}/fonyx/quickshell"
rm -rf "$share"
mkdir -p "$share"
cp -r quickshell/. "$share/"
sed "s|/usr/share/fonyx/quickshell|$share|" \
  packaging/systemd/fonyx-quickshell.service >"$unit_dir/fonyx-quickshell.service"

# A hand-started daemon would hold the D-Bus name and block the service.
if ! systemctl --user is-active --quiet fonyx-daemon.service; then
  # -f, not -x: process names are cut to 15 characters, and this one is 17.
  pkill -f '(^|/)fonyx-daemon$' || true
fi

systemctl --user daemon-reload
systemctl --user enable fonyx-daemon.service fonyx-ui.service
# Clear a start-limit left by earlier failed attempts, then (re)start.
systemctl --user reset-failed fonyx-daemon.service fonyx-quickshell.service 2>/dev/null || true
systemctl --user restart fonyx-daemon.service
systemctl --user try-restart fonyx-quickshell.service fonyx-ui.service
systemctl --user --no-pager status fonyx-daemon.service | head -n 5
