#!/bin/sh
# Runs the UI and endpoint tests against a throwaway PipeWire + D-Bus +
# daemon, so they never touch the real audio session or the real config.
# CI runs exactly this; locally it is the safe way to run the suite.
#
#   sh ui/tests/run-isolated.sh [playwright args...]
#
# FONYX_TEST_CMD runs something else in that session instead of
# Playwright (from the repository root), e.g. the CLI's own check:
#   FONYX_TEST_CMD="sh cli/tests/smoke.sh" sh ui/tests/run-isolated.sh
#
# Needs: pipewire, wireplumber, dbus-run-session, a built daemon
# (cargo build -p fonyx-daemon) and, for Playwright, `npm ci` done in
# ui/tests.
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
daemon=${FONYX_DAEMON:-$root/target/debug/fonyx-daemon}
[ -x "$daemon" ] || { echo "no daemon binary at $daemon (cargo build -p fonyx-daemon)" >&2; exit 1; }

tmp=$(mktemp -d)
# FONYX_RUNTIME_DIR: testing the snap, whose AppArmor profile only allows
# the PipeWire and D-Bus sockets under /run/user/<uid>.
export XDG_RUNTIME_DIR="${FONYX_RUNTIME_DIR:-$tmp/run}" XDG_CONFIG_HOME="$tmp/config" XDG_STATE_HOME="$tmp/state"
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
  <listen>unix:path=$XDG_RUNTIME_DIR/bus</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
EOF

export FONYX_ISOLATED=1 FONYX_ROOT="$root" FONYX_DAEMON_BIN="$daemon" FONYX_TMP="$tmp"
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
    rm -rf "$FONYX_TMP"
  }
  trap cleanup EXIT
  wait_for() { i=0; until "$@" >/dev/null 2>&1; do i=$((i + 1)); [ "$i" -gt 100 ] && { echo "timed out waiting for: $*" >&2; exit 1; }; sleep 0.1; done; }

  pipewire > "$FONYX_TMP/pipewire.log" 2>&1 & pids="$pids $!"
  wait_for test -S "$XDG_RUNTIME_DIR/pipewire-0"
  wireplumber > "$FONYX_TMP/wireplumber.log" 2>&1 & pids="$pids $!"

  # One fake output device and two fake apps (silent, but real graph nodes).
  pw-cli create-node adapter "{ factory.name=support.null-audio-sink node.name=test-output node.description=\"Test Output\" media.class=Audio/Sink object.linger=true audio.position=[FL FR] }" >/dev/null
  for app in Alpha Beta; do
    pw-cli create-node adapter "{ factory.name=support.null-audio-sink node.name=test-app-$app application.name=TestApp$app media.class=Stream/Output/Audio object.linger=true audio.position=[FL FR] }" >/dev/null
  done

  RUST_LOG=${RUST_LOG:-warn} "$FONYX_DAEMON_BIN" > "$FONYX_TMP/daemon.log" 2>&1 & daemon_pid=$! pids="$pids $daemon_pid"
  wait_for busctl --user status org.fonyx.Daemon

  status=0
  if [ -n "${FONYX_TEST_CMD:-}" ]; then
    cd "$FONYX_ROOT"
    sh -c "$FONYX_TEST_CMD" || status=$?
  else
    cd "$FONYX_ROOT/ui/tests"
    npx playwright test "$@" || status=$?
  fi
  if [ "$status" -ne 0 ]; then
    echo "--- daemon log (tail)"; tail -n 40 "$FONYX_TMP/daemon.log"
    # A daemon that died mid-run fails every later test; say how it went
    # (139 = segfault, 134 = abort, 0 = it quit by itself, e.g. PipeWire left).
    if ! kill -0 "$daemon_pid" 2>/dev/null; then
      st=0; wait "$daemon_pid" || st=$?
      echo "--- daemon exited during the run, status $st"
      for log in pipewire wireplumber; do echo "--- $log log (tail)"; tail -n 20 "$FONYX_TMP/$log.log"; done
    fi
  fi
  exit "$status"
' sh "$@"
