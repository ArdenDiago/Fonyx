#!/bin/sh
# Runs the UI and endpoint tests against a throwaway PipeWire + D-Bus +
# daemon, so they never touch the real audio session or the real config.
# CI runs exactly this; locally it is the safe way to run the suite.
#
#   sh ui/tests/run-isolated.sh [playwright args...]
#
# Needs: pipewire, wireplumber, dbus-run-session, a built daemon
# (cargo build -p mtxpanel-daemon) and `npm ci` done in ui/tests.
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
daemon=${MTX_DAEMON:-$root/target/debug/mtxpanel-daemon}
[ -x "$daemon" ] || { echo "no daemon binary at $daemon (cargo build -p mtxpanel-daemon)" >&2; exit 1; }

tmp=$(mktemp -d)
export XDG_RUNTIME_DIR="$tmp/run" XDG_CONFIG_HOME="$tmp/config" XDG_STATE_HOME="$tmp/state"
mkdir -p "$XDG_RUNTIME_DIR" "$XDG_STATE_HOME" "$XDG_CONFIG_HOME/wireplumber/wireplumber.conf.d"
chmod 700 "$XDG_RUNTIME_DIR"
# A second session manager must not open the machine's real devices.
# (WirePlumber 0.5 syntax; 0.4 ignores it, and CI runners have no devices.)
cat > "$XDG_CONFIG_HOME/wireplumber/wireplumber.conf.d/10-no-hardware.conf" <<'EOF'
wireplumber.profiles = {
  main = {
    monitor.alsa = disabled
    monitor.alsa-midi = disabled
    monitor.bluez = disabled
    monitor.bluez-midi = disabled
    monitor.v4l2 = disabled
    monitor.libcamera = disabled
  }
}
EOF

# A bare session bus: no service activation, so nothing from the desktop
# (portals, gvfs, ...) gets started inside the throwaway session.
cat > "$tmp/bus.conf" <<EOF
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:dir=$tmp</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
EOF

export MTX_ISOLATED=1 MTX_ROOT="$root" MTX_DAEMON_BIN="$daemon" MTX_TMP="$tmp"
# The session script is single-quoted on purpose: it expands inside the
# private D-Bus session, not here.
# shellcheck disable=SC2016
dbus-run-session --config-file="$tmp/bus.conf" -- sh -c '
  pids=""
  # Newest first (daemon before PipeWire), then make sure: the daemon waits
  # on PipeWire to finish a graceful stop and would hang if it went first.
  cleanup() {
    for p in $(echo "$pids" | tr " " "\n" | tac); do kill "$p" 2>/dev/null || true; done
    sleep 0.3
    for p in $pids; do kill -9 "$p" 2>/dev/null || true; done
    rm -rf "$MTX_TMP"
  }
  trap cleanup EXIT
  wait_for() { i=0; until "$@" >/dev/null 2>&1; do i=$((i + 1)); [ "$i" -gt 100 ] && { echo "timed out waiting for: $*" >&2; exit 1; }; sleep 0.1; done; }

  pipewire > "$MTX_TMP/pipewire.log" 2>&1 & pids="$pids $!"
  wait_for test -S "$XDG_RUNTIME_DIR/pipewire-0"
  wireplumber > "$MTX_TMP/wireplumber.log" 2>&1 & pids="$pids $!"

  # One fake output device and two fake apps (silent, but real graph nodes).
  pw-cli create-node adapter "{ factory.name=support.null-audio-sink node.name=test-output node.description=\"Test Output\" media.class=Audio/Sink object.linger=true audio.position=[FL FR] }" >/dev/null
  for app in Alpha Beta; do
    pw-cli create-node adapter "{ factory.name=support.null-audio-sink node.name=test-app-$app application.name=TestApp$app media.class=Stream/Output/Audio object.linger=true audio.position=[FL FR] }" >/dev/null
  done

  RUST_LOG=${RUST_LOG:-warn} "$MTX_DAEMON_BIN" > "$MTX_TMP/daemon.log" 2>&1 & pids="$pids $!"
  wait_for busctl --user status org.mtxpanel.Daemon

  cd "$MTX_ROOT/ui/tests"
  status=0
  npx playwright test "$@" || status=$?
  if [ "$status" -ne 0 ]; then echo "--- daemon log (tail)"; tail -n 40 "$MTX_TMP/daemon.log"; fi
  exit "$status"
' sh "$@"
