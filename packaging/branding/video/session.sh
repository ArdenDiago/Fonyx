#!/bin/sh
# Runs inside ui/tests/run-isolated.sh (a throwaway PipeWire + D-Bus + daemon) and films the real
# UI there: three fake devices, four apps playing real audio, the groups the film talks about.
#
#     FONYX_TEST_CMD="sh packaging/branding/video/session.sh <take|all|probe>" sh ui/tests/run-isolated.sh
set -eu
here=packaging/branding/video
fx="target/debug/fonyx -q"
wait_for() { i=0; until "$@" >/dev/null 2>&1; do i=$((i + 1)); [ "$i" -gt 100 ] && { echo "timed out: $*" >&2; exit 1; }; sleep 0.1; done; }

# run-isolated.sh's own fake output and silent apps would show up in the shots
for n in test-output test-app-Alpha test-app-Beta; do
  id=$(pw-dump | jq -r ".[] | select(.info.props[\"node.name\"] == \"$n\") | .id")
  [ -n "$id" ] && pw-cli destroy "$id" >/dev/null || true
done
# Restart the daemon without realtime scheduling. While the browser was screencasting it was
# SIGKILLed (exit 137) in most runs; its RT data-loop thread runs under RLIMIT_RTTIME = 200 ms,
# the likely culprit. Latency does not matter for filming.
mkdir -p "$XDG_CONFIG_HOME/pipewire/client.conf.d"
echo 'context.properties = { module.rt = false }' > "$XDG_CONFIG_HOME/pipewire/client.conf.d/90-no-rt.conf"
old=$(busctl --user status org.fonyx.Daemon | sed -n 's/^PID=//p')
kill "$old"; while kill -0 "$old" 2>/dev/null; do sleep 0.1; done
"$FONYX_DAEMON_BIN" > "$FONYX_TMP/daemon-film.log" 2>&1 & daemon=$!
trap 'kill $daemon 2>/dev/null' EXIT
wait_for busctl --user status org.fonyx.Daemon

for d in "headphones:Studio Headphones" "speakers:Desk Speakers" "hdmi:HDMI TV"; do
  pw-cli create-node adapter "{ factory.name=support.null-audio-sink node.name=${d%%:*} node.description=\"${d#*:}\" media.class=Audio/Sink object.linger=true audio.position=[FL FR] }" >/dev/null
done
sleep 0.5

# app name : group : source file
# app name : group : what it is playing : source file (media.name, or the UI shows the file path)
for a in "Music Player:Music:Night Drive:src-music.wav" "Web Browser:Browser:Video:src-browser.wav" "Voice Call:Call:Team call:src-call.wav" "Narrator:Voice:Narration:src-eqvoice.wav"; do
  name=${a%%:*}; rest=${a#*:}; group=${rest%%:*}; rest=${rest#*:}; title=${rest%%:*}; file=${rest#*:}
  $fx group create "$group" >/dev/null || true
  pw-play -P "{ application.name = \"$name\" node.name = \"$(echo "$name" | tr ' A-Z' '-a-z')\" media.name = \"$title\" }" "$here/footage/$file" &
done
# a fifth app nobody has sorted yet: the groups take puts it in a group of its own
pw-play -P '{ application.name = "Game" node.name = "game" media.name = "Level 3" }' "$here/footage/src-browser.wav" &
wait_for sh -c "target/debug/fonyx --json input ls | jq -e 'length >= 5'"
for a in "Music Player:Music" "Web Browser:Browser" "Voice Call:Call" "Narrator:Voice"; do
  $fx input move "${a%%:*}" "${a#*:}"
done
$fx output default "Desk Speakers"
$fx mtx set Music "Studio Headphones"
$fx mtx set Call "Desk Speakers"
$fx mtx set Call "Studio Headphones"
$fx mtx set Browser "Desk Speakers"
$fx mtx set Voice "Desk Speakers"
$fx fader set Music -2dB
$fx fader set Browser -4dB

# capture.mjs expects the bridge on 4571; a bridge left over from an aborted run would answer
# there from a dead session bus
curl -sf http://localhost:4571/ >/dev/null && { echo "port 4571 is taken (a stale ui/tests/bridge.mjs?)" >&2; exit 1; }
BRIDGE_PORT=4571 node ui/tests/bridge.mjs >/dev/null 2>&1 & bridge=$!
trap 'kill $daemon $bridge 2>/dev/null' EXIT
wait_for curl -sf http://localhost:4571/
node "$here/capture.mjs" "$@"
