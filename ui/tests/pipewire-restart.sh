#!/bin/sh
# PipeWire restarting under a running daemon (a crash, an update,
# `systemctl --user restart pipewire`): the daemon must reconnect, rebuild
# its nodes and keep answering D-Bus throughout. Runs only in the
# throwaway session:
#
#   FONYX_TEST_CMD="sh ui/tests/pipewire-restart.sh" sh ui/tests/run-isolated.sh
set -eu
[ -n "${FONYX_ISOLATED:-}" ] || { echo "only runs inside ui/tests/run-isolated.sh" >&2; exit 1; }

# This session's processes only: match its private XDG_RUNTIME_DIR, never
# the machine's real PipeWire.
mine() { for p in $(pgrep -x "$1"); do tr '\0' '\n' < "/proc/$p/environ" 2>/dev/null | grep -qx "XDG_RUNTIME_DIR=$XDG_RUNTIME_DIR" && echo "$p"; done; }
fail() { echo "FAIL: $*" >&2; tail -n 20 "$FONYX_TMP/daemon.log" >&2; exit 1; }
daemon=$(busctl --user status org.fonyx.Daemon | sed -n 's/^PID=//p')
nodes() { pw-cli ls Node 2>/dev/null | grep -c 'node.name = "fonyx.group.' || true; }

[ "$(nodes)" -ge 2 ] || fail "no group nodes before the restart"
kill $(mine wireplumber) $(mine pipewire)
sleep 1
kill -0 "$daemon" || fail "daemon died with PipeWire"
busctl --user call org.fonyx.Daemon /org/fonyx/Daemon org.fonyx.Daemon ListGroups >/dev/null || fail "D-Bus stopped answering while PipeWire was down"

pipewire > "$FONYX_TMP/pipewire2.log" 2>&1 &
sleep 0.5
wireplumber > "$FONYX_TMP/wireplumber2.log" 2>&1 &
i=0; until [ "$(nodes)" -ge 2 ]; do i=$((i + 1)); [ "$i" -gt 50 ] && fail "group nodes never came back"; sleep 0.2; done
kill -0 "$daemon" || fail "daemon restarted instead of reconnecting"
echo "pipewire restart: daemon reconnected and rebuilt its nodes"
# The runner's cleanup only knows the first PipeWire.
kill $(mine wireplumber) $(mine pipewire) 2>/dev/null || true
