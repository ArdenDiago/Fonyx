# Command line

`fonyx` controls the mixer from a terminal or a script. It is a thin
client, like the UI: every command is one or a few calls on the daemon's
[D-Bus interface](dbus.md), and the `system` service commands run
`systemctl --user`. It holds no audio logic and no state of its own, apart
from the alias file.

The source is in `cli/`. The D-Bus proxy it uses is the `client/` crate,
which the Tauri UI shares.

```bash
cargo run -p fonyx-cli -- group ls   # from a checkout
fonyx group ls                       # after packaging/install-user.sh
```

## Shape

```text
fonyx [--json] [--quiet] <noun> <verb> [arguments]
fonyx --version | --help
```

Every noun and every verb takes `--help`. Verbs can be shortened as long as
they stay unambiguous (`fonyx group l`).

| Option | Effect |
|---|---|
| `--json` | Print JSON instead of text. Lists are arrays of objects; a command that creates something prints `{"id": ...}`. |
| `-q`, `--quiet` | Print nothing on success. |

A command exits 0 on success and 1 on failure, with the reason on standard
error. `system status` also exits 1 when the daemon does not answer, and
`system doctor` when a health check is an error. A usage mistake exits 2.

## Naming things

Groups, inputs, outputs, scenes and rules can be named four ways. The first
that matches wins:

1. the id (`group show 9`)
2. the exact name, in any letter case (`group show firefox`)
3. the start of a name (`group show fire`), then any part of it
4. an alias (see [Aliases](#aliases))

A partial name has to point at one thing; otherwise the command stops and
lists the candidates with their ids. An input's name can be its application
(`Firefox`), its site or service (`YouTube`) or its app id
(`appname:Firefox`). Several streams of one application share a name, and a
command given that name acts on all of them.

`master` stands for the master bus wherever a command takes "a group or
master". `none` clears a setting where the help says so.

## Levels

Anywhere a level is expected:

| Written | Means |
|---|---|
| `75%` | 0.75 of unity |
| `-6dB` | decibels relative to unity |
| `0.75` | a linear multiplier, as the daemon stores it |

Levels above +12 dB are refused, which also catches `75` typed for `75%`.

## Commands

### `system`

| Command | Does |
|---|---|
| `system status` | Service states, CPU and RAM, buffer, underruns, health |
| `system start [backend\|frontend]` | Start the daemon and the UI, or one of them |
| `system stop [backend\|frontend]` | Stop the UI and the daemon, or one of them |
| `system restart [backend\|frontend]` | Restart them |
| `system doctor` | The health checks, one per line |
| `system graph` | The signal-flow graph: nodes and links |
| `system logs [-f] [-n N]` | The daemon's journal |
| `system set buffer-ms\|max-compensation-ms <value>` | An engine setting; prints the value the daemon kept after clamping |

`backend` is `fonyx-daemon.service`. `frontend` is
`fonyx-ui.service`, plus `fonyx-quickshell.service` when that is
in use. These are the units `packaging/install-user.sh` installs.

Starting or restarting the backend first clears what would block it: a
daemon started by hand (it holds the D-Bus name the service needs) is
stopped, and a start limit left by earlier crashes is reset. The command
returns once the daemon answers, so `fonyx system restart && fonyx
scene apply Night` works.

### `group`

| Command | Does |
|---|---|
| `group ls` | Groups with level, input count and kind |
| `group show <group>` | Level, pan, inputs, filters, dynamics, EQ, routes, plugins |
| `group create <name>` | Create a group |
| `group rm <group>` | Delete it; its inputs return to Default |
| `group rename <group> <name>` | Rename it |
| `group feed <group> <parent\|none>` | Make it a sub-group of another group |

Default and Cue cannot be deleted or renamed. A rename shows at once in every
client; the group's PipeWire sink keeps its old description (the name
`pavucontrol` shows) until the daemon restarts.

### `input`

| Command | Does |
|---|---|
| `input ls` | Application streams, their group and side |
| `input move <input> <group>` | Move into a group. The daemon also remembers it as the app's group. |
| `input unroute <input>` | Back to Default |
| `input mute <input>` / `input unmute <input>` | Mute the stream itself |
| `input side <input> L\|R\|stereo` | Send it to one channel of its group |
| `input strip <input>` | "Own DSP": give the app a group of its own |
| `input profile save <input> <label>` | Remember the app's group and volume |
| `input levels [--watch]` | Per-input peak meters |

### `fader`

| Command | Does |
|---|---|
| `fader ls` | Master and every group: level, pan, mono |
| `fader set <group\|master> <level>` | Set a level |
| `fader pan <group> [-1..1] [--mono\|--stereo]` | Pan, and mono or stereo; with nothing else, prints them |
| `fader levels [--watch]` | Peaks and compressor / gate reduction |

### `dsp`

Each of these prints the current settings when given no options, and
changes only the options that are given.

| Command | Does |
|---|---|
| `dsp eq <group> [band] [--freq] [--gain] [--q] [--on\|--off]` | The EQ. Bands count from 1. |
| `dsp eq-level <group> [dB]` | Move the whole curve, -18 to 18 dB |
| `dsp hpf <group> [--freq] [--q] [--on\|--off]` | High-pass filter |
| `dsp lpf <group> [--freq] [--q] [--on\|--off]` | Low-pass filter |
| `dsp comp <group> [--threshold] [--ratio] [--attack] [--release] [--makeup] [--on\|--off]` | Compressor |
| `dsp gate <group> [--threshold] [--attack] [--release] [--hold] [--on\|--off]` | Gate |
| `dsp plugin available` | Plugin types and their parameters |
| `dsp plugin ls <group>` | The group's chain |
| `dsp plugin add <group> <plugin>` | Append a plugin |
| `dsp plugin rm <group> <index>` | Remove a slot |
| `dsp plugin bypass <group> <index> [--off]` | Bypass a slot, or put it back |
| `dsp plugin set <group> <index> <param> <value>` | Set a parameter |
| `dsp plugin move <group> <from> <to>` | Reorder |

Changing a parameter does not switch a stage on: `dsp hpf Music --freq 80`
leaves a disabled filter disabled. Add `--on`. Plugin slots are numbered as
`dsp plugin ls` shows them, from 0.

### `mtx`

The matrix of buses and outputs.

| Command | Does |
|---|---|
| `mtx ls [group\|master]` | Routes of every bus, or of one |
| `mtx set <group\|master> <output> [--gain <level>] [--on\|--off]` | Add a route, or change one |
| `mtx rm <group\|master> <output>` | Remove a route |
| `mtx sync <group\|master> <output> off\|auto\|manual [delay-ms]` | Delay compensation for a route |

A group with no routes plays on the default output. The Cue group's routes
are the cue outputs and are changed with `output cue`.

### `output`

| Command | Does |
|---|---|
| `output ls` | Devices, their own volume, default and cue flags |
| `output volume <output> [level]` | Show or set a device's volume |
| `output mute <output>` / `output unmute <output>` | Mute a device |
| `output default [output\|none]` | The device for groups with no routes |
| `output cue ls` | The cue outputs |
| `output cue add <output>` / `output cue rm <output>` | Change them |
| `output cue duck [level]` | Level of the rest of the mix on cue outputs while cueing |

### `scene`, `rule`, `duck`, `record`

| Command | Does |
|---|---|
| `scene ls` | Saved scenes |
| `scene save <name>` | Snapshot the current state |
| `scene update <scene>` | Overwrite a scene with the current state |
| `scene dup <scene> <name>` | Copy a scene |
| `scene rm <scene>` / `scene apply <scene>` | Delete or apply |
| `rule ls` / `rule history` | Rules, and their recent firings |
| `rule show <rule>` | One rule as JSON |
| `rule save <file\|->` | Create or replace a rule from JSON, the format `rule show` prints |
| `rule enable <rule>` / `rule disable <rule>` / `rule rm <rule>` | Manage a rule |
| `duck ls` | Ducking rules |
| `duck create <trigger> <target>... [--reduction] [--attack] [--hold] [--release] [--threshold]` | Turn the targets down while the trigger group is heard |
| `duck enable <id>` / `duck disable <id>` / `duck rm <id>` | Manage a ducking rule |
| `record ls` | Recordings in progress |
| `record start <group> <path> [--format wav16\|wav32\|flac]` | Record a group. A `.flac` path records FLAC, anything else 16-bit WAV. |
| `record stop <id>` | Stop and finish the file |

`rule save` with an id of 4294967295 creates a rule; with an existing id it
replaces that rule. The two rule templates (`CreateAutoRouteRule`,
`CreateDeviceConnectedRule`) have no command; write the rule as JSON.

## Aliases

Output ids are long and application names repeat. An alias is a short name
of your own:

```bash
fonyx alias set cans "Chu2 DSP"        # an output
fonyx alias set ff firefox             # an input: the application
fonyx alias set bgm Music              # a group
fonyx alias ls
fonyx alias rm cans

fonyx mtx set master cans --gain -3dB
fonyx input move ff bgm
```

`alias set` works out whether the target is an input, an output or a group;
if it could be more than one, name it with `--kind`. The target has to exist
when the alias is made. What is stored is the identity that survives a
restart: an output's id, a group's id, an application's app id (so an input
alias means every stream of that application, including future ones).

An alias only applies where its kind is expected, and it is tried before
names. Aliases live in `~/.config/fonyx/aliases.toml`, beside the
daemon's `config.toml`. The daemon and the UI never read them.

## Scripting

```bash
# Every group's level, as name<TAB>dB
fonyx --json fader ls | jq -r '.[] | [.name, .db] | @tsv'

# Create a group, route an output to it, keep the id
id=$(fonyx --json group create Podcast | jq .id)
fonyx -q mtx set "$id" cans --gain 80%

# Only apply a scene when the daemon is healthy
fonyx -q system doctor >/dev/null && fonyx scene apply Night
```

Text output is for reading and may change; scripts should use `--json`. In
JSON, levels are linear and silence in a `*_db` field is `null`.

## Tests

`cli/src/` has unit tests for name resolution, level parsing and the text
layout (`cargo test -p fonyx-cli`). `cli/tests/smoke.sh` runs every
command against a real daemon in the isolated session; see
[Testing](testing.md#cli-tests).
