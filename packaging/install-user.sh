#!/bin/sh
# Installs the daemon as a systemd *user* service without root: starts at
# login, restarts on crash or hang (see the unit's watchdog). Re-run after
# pulling changes to install the new build.
set -eu
cd "$(dirname "$0")/.."

cargo build --release -p sonicforge-daemon -p sonicforge-ui

# Copy then rename: overwriting a running binary in place fails (ETXTBSY).
mkdir -p "$HOME/.local/bin"
install -m755 target/release/sonicforge-daemon "$HOME/.local/bin/sonicforge-daemon.new"
mv -f "$HOME/.local/bin/sonicforge-daemon.new" "$HOME/.local/bin/sonicforge-daemon"
install -m755 target/release/sonicforge-ui "$HOME/.local/bin/sonicforge-ui.new"
mv -f "$HOME/.local/bin/sonicforge-ui.new" "$HOME/.local/bin/sonicforge-ui"

unit_dir="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
mkdir -p "$unit_dir"

# Retire an install made under the project's old name, so two daemons never
# compete for the audio graph. Its config is moved over by the new daemon.
for old in mtxpanel-ui mtxpanel-quickshell mtxpanel-daemon; do
    systemctl --user disable --now "$old.service" 2>/dev/null || true
    rm -f "$unit_dir/$old.service" "$HOME/.local/bin/$old"
done
pkill -x mtxpanel-daemon 2>/dev/null || true
rm -rf "${XDG_DATA_HOME:-$HOME/.local/share}/mtxpanel"
sed 's|^ExecStart=/usr/bin/sonicforge-daemon|ExecStart=%h/.local/bin/sonicforge-daemon|' \
    packaging/systemd/sonicforge-daemon.service > "$unit_dir/sonicforge-daemon.service"
# The Tauri UI is what opens at login.
sed 's|^ExecStart=/usr/bin/sonicforge-ui|ExecStart=%h/.local/bin/sonicforge-ui|' \
    packaging/systemd/sonicforge-ui.service > "$unit_dir/sonicforge-ui.service"

# Quickshell files + its unit (the packaged unit expects /usr/share/sonicforge).
# Not enabled here (Settings tab toggle). SonicForgePopup.qml is the fader
# popup meant for a compositor keybind, e.g. niri:
#   Mod+A { spawn-sh "pkill -f SonicForgePopup.qml || quickshell -p ~/.local/share/sonicforge/quickshell/SonicForgePopup.qml"; }
share="${XDG_DATA_HOME:-$HOME/.local/share}/sonicforge/quickshell"
rm -rf "$share"
mkdir -p "$share"
cp -r quickshell/. "$share/"
sed "s|/usr/share/sonicforge/quickshell|$share|" \
    packaging/systemd/sonicforge-quickshell.service > "$unit_dir/sonicforge-quickshell.service"

# A hand-started daemon would hold the D-Bus name and block the service.
if ! systemctl --user is-active --quiet sonicforge-daemon.service; then
    # -f, not -x: process names are cut to 15 characters, and this one is 17.
    pkill -f '(^|/)sonicforge-daemon$' || true
fi

systemctl --user daemon-reload
systemctl --user enable sonicforge-daemon.service sonicforge-ui.service
# Clear a start-limit left by earlier failed attempts, then (re)start.
systemctl --user reset-failed sonicforge-daemon.service sonicforge-quickshell.service 2>/dev/null || true
systemctl --user restart sonicforge-daemon.service
systemctl --user try-restart sonicforge-quickshell.service sonicforge-ui.service
systemctl --user --no-pager status sonicforge-daemon.service | head -n 5
