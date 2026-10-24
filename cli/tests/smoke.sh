#!/bin/sh
# End-to-end check of the `fonyx` CLI against a real daemon. It creates,
# routes, mutes and deletes things, so it only runs in the throwaway session:
#
#   cargo build -p fonyx-daemon -p fonyx-cli
#   FONYX_TEST_CMD="sh cli/tests/smoke.sh" sh ui/tests/run-isolated.sh
#
# That session has one fake output ("Test Output") and two fake apps
# (TestAppAlpha, TestAppBeta).
set -eu
[ -n "${FONYX_ISOLATED:-}" ] || { echo "only runs inside ui/tests/run-isolated.sh (see the header)" >&2; exit 1; }

bin="${FONYX_BIN:-$FONYX_ROOT/target/debug/fonyx}"
checks=0
s() { "$bin" "$@"; }
# has <text> <fonyx args...>: the command succeeds and prints <text>.
has() {
  want=$1; shift; checks=$((checks + 1))
  got=$(s "$@" 2>&1) || { echo "FAIL: fonyx $* exited non-zero: $got" >&2; exit 1; }
  printf '%s\n' "$got" | grep -qF -- "$want" || { echo "FAIL: fonyx $*: no \"$want\" in: $got" >&2; exit 1; }
}
# lacks <text> <fonyx args...>: the command succeeds and does not print <text>.
lacks() {
  want=$1; shift; checks=$((checks + 1))
  got=$(s "$@" 2>&1) || { echo "FAIL: fonyx $* exited non-zero: $got" >&2; exit 1; }
  if printf '%s\n' "$got" | grep -qF -- "$want"; then echo "FAIL: fonyx $*: unexpected \"$want\" in: $got" >&2; exit 1; fi
}
# fails <text> <fonyx args...>: the command fails and says <text>.
fails() {
  want=$1; shift; checks=$((checks + 1))
  if got=$(s "$@" 2>&1); then echo "FAIL: fonyx $* should have failed: $got" >&2; exit 1; fi
  printf '%s\n' "$got" | grep -qF -- "$want" || { echo "FAIL: fonyx $*: no \"$want\" in: $got" >&2; exit 1; }
}

# --- basics
has "fonyx 0." --version
has "Usage: fonyx" --help
has "daemon: answering" system status
has "score:" system doctor
has "nodes:" system graph
has "buffer_ms: 40" system set buffer-ms 40
has "buffer_ms: 150" system set buffer-ms 9999   # the daemon clamps

# --- groups
id=$(s --json group create Music | jq .id)
has "Music" group ls
has "renamed group Music to Tunes" group rename "$id" Tunes
has "name: Tunes" group show tun              # by prefix
fails "cannot be deleted" group rm Default
fails "cannot be renamed" group rename Default Main
fails "no group matches" group show nosuchgroup
has "created group Sub" group create Sub
has "Sub now feeds Tunes" group feed Sub Tunes
has "Tunes" group ls                          # FEEDS column
has "plays on its outputs again" group feed Sub none

# --- inputs
has "TestAppAlpha" input ls
has "moved 1 input(s) to Tunes" input move testappalpha Tunes
[ "$(s --json input ls | jq -r '.[] | select(.name == "TestAppAlpha") | .group')" = Tunes ] || { echo "FAIL: input not in Tunes" >&2; exit 1; }
fails "several inputs" input move testapp Tunes   # Alpha and Beta both start so
has "left channel" input side testappalpha L
has "stereo" input side testappalpha stereo
has "muted 1 input(s)" input mute testappbeta
has "unmuted 1 input(s)" input unmute testappbeta
has "TestAppBeta" input levels
has "back to Default" input unroute testappalpha

# --- faders
has "Tunes at -6.0 dB" fader set Tunes -6dB
has "-6.0" fader ls
has "master at -6.0 dB" fader set master 50%
has "master at +0.0 dB" fader set master 1
fails "write 75%" fader set Tunes 75
has "pan -0.5, mono" fader pan Tunes -0.5 --mono
has "pan: -0.5" fader pan Tunes
has "pan -0.5, stereo" fader pan Tunes --stereo
fails "from -1" fader pan Tunes 3
has "NAME" fader levels

# --- dsp
has "300.0    3.0      1.0  yes" dsp eq Tunes 2 --gain 3 --q 1 --on
has "BAND" dsp eq Tunes
fails "no band 99" dsp eq Tunes 99 --gain 1
fails "which band" dsp eq Tunes --gain 1
has "eq_level_db: -3" dsp eq-level Tunes -3
has "freq_hz: 80.0" dsp hpf Tunes --freq 80 --on
has "enabled: yes" dsp hpf Tunes
has "enabled: no" dsp lpf Tunes --off
has "ratio: 4.0" dsp comp Tunes --threshold -20 --ratio 4 --on
fails "1 or more" dsp comp Tunes --ratio 0.5
has "threshold_db: -50.0" dsp gate Tunes --threshold -50 --hold 20
fails "not a number" dsp hpf Tunes --freq nan
has "soft_clipper" dsp plugin available
has "added soft_clipper" dsp plugin add Tunes soft_clipper
has "added stereo_widener" dsp plugin add Tunes stereo_widener
fails "no plugin type" dsp plugin add Tunes reverb9000
has "soft_clipper drive = 3" dsp plugin set Tunes 0 drive 3
fails "no parameter" dsp plugin set Tunes 0 nope 1
has "bypassed" dsp plugin bypass Tunes 0
has "back in the chain" dsp plugin bypass Tunes 0 --off
has "moved soft_clipper to slot 1" dsp plugin move Tunes 0 1
has "removed stereo_widener" dsp plugin rm Tunes 0
fails "no plugin slot 7" dsp plugin rm Tunes 7
has "drive=3" dsp plugin ls Tunes

# --- outputs, aliases and the matrix
has "Test Output" output ls
has "volume: 0.5" output volume "test output" 50%
has "muted" output mute test
has "unmuted" output unmute test
has "to = output" alias set to "Test Output"
has "to" alias ls
fails "cannot be an alias" alias set master Tunes
fails "is not an input, output or group" alias set x nosuchthing
has "= input appname:TestAppAlpha" alias set alpha testappalpha
has "moved 1 input(s) to Tunes" input move alpha Tunes
has "route on, -6.0 dB" mtx set Tunes to --gain 50%
has "Test Output" mtx ls Tunes
has "Tunes" mtx ls
has "route off, -6.0 dB" mtx set Tunes to --off   # the gain is kept
has "sync manual" mtx sync Tunes to manual 20
has "20.0" mtx ls Tunes
has "route removed" mtx rm Tunes to
fails "no such route" mtx rm Tunes to
has "route on" mtx set master to
has "master" mtx ls
has "route removed" mtx rm master to
fails "cue outputs" mtx set Cue to
has "unrouted groups now play there" output default to
has "default: Test Output" output default
has "no preferred output" output default none
has "cue output added" output cue add to
has "Test Output" output cue ls
has "cue output removed" output cue rm to
has "cue_duck_level: 0.3" output cue duck 30%
has "removed alias alpha" alias rm alpha
fails "no alias" alias rm alpha

# --- scenes
has "created scene Night" scene save Night
has "Night" scene ls
has "scene updated" scene update night
has "created scene Late" scene dup Night Late
has "scene applied" scene apply Late
has "scene deleted" scene rm Late
lacks "Late" scene ls

# --- rules
rule="{\"id\":4294967295,\"name\":\"CliRule\",\"enabled\":true,\"priority\":1,\"cooldown_ms\":0,\"trigger\":{\"AppStarted\":{\"app_id\":\"appname:Nobody\",\"service\":null}},\"conditions\":[],\"actions\":[{\"RouteTriggeringAppToGroup\":{\"group_id\":$id}}]}"
printf '%s' "$rule" | s rule save - >/dev/null
has "CliRule" rule ls
has '"name": "CliRule"' rule show clirule
has "rule disabled" rule disable CliRule
has "rule enabled" rule enable CliRule
s rule show CliRule | s rule save - >/dev/null   # show's output is what save takes
has "(none)" rule history
has "rule deleted" rule rm CliRule
fails "no rule matches" rule rm CliRule

# --- ducking
duck=$(s --json duck create Tunes Default --reduction 9 | jq .id)
has "Tunes" duck ls
fails "cannot duck itself" duck create Tunes Tunes
has "ducking rule disabled" duck disable "$duck"
has "ducking rule deleted" duck rm "$duck"
fails "no ducking rule" duck rm "$duck"

# --- recording
rec=$(s --json record start Tunes "$FONYX_TMP/take.flac" | jq .id)
has "take.flac" record ls
has "recording stopped" record stop "$rec"
fails "no recording" record stop "$rec"
# `record stop` returns once the engine has been told; the file is finished and
# renamed into place a moment later. Checking at once failed on a slow CI runner.
checks=$((checks + 1)); n=0
until [ -f "$FONYX_TMP/take.flac" ]; do
    n=$((n + 1)); [ "$n" -lt 50 ] || { echo "FAIL: no recording file" >&2; exit 1; }
    sleep 0.1
done

# --- scripting: --quiet says nothing, --json is JSON
checks=$((checks + 1)); [ -z "$(s -q fader set Tunes 0dB)" ] || { echo "FAIL: --quiet printed something" >&2; exit 1; }
checks=$((checks + 1)); s --json group ls | jq -e 'type == "array" and (.[0] | has("id"))' >/dev/null

# --- cleanup, and what deleting leaves behind
has "deleted group Sub" group rm Sub
has "deleted group Tunes" group rm "$id"
lacks "Tunes" group ls

echo "cli smoke: $checks checks passed"
