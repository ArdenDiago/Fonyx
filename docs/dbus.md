# D-Bus Interface

The daemon exposes one interface on the **session bus**. It is the only way
clients control the daemon.

| | |
|---|---|
| Service | `org.mtxpanel.Daemon` |
| Object path | `/org/mtxpanel/Daemon` |
| Interface | `org.mtxpanel.Daemon` |

The interface is defined in `daemon/src/dbus.rs` with `zbus`. Method names
on the bus are the PascalCase form of the Rust function names. The source is
the authority; the tables below were derived from it.

To see the live interface with full signatures:

```bash
busctl --user introspect org.mtxpanel.Daemon /org/mtxpanel/Daemon
```

## Conventions

- **Numbers.** Ids are `u32` (`u`). Rust `f32` values travel as D-Bus
  doubles (`d`).
- **Gain** is a linear multiplier (1.0 is unity) unless a parameter name
  ends in `_db`.
- **Structured replies** are positional tuples, not dictionaries. Field
  order is given in each table.
- **Not found** is signalled in-band: `u32::MAX` for an id, an empty string
  or an empty list for data.
- **Output ids and app ids** are stable strings, not PipeWire node ids.
  Stream ids are PipeWire node ids and are valid only while the stream
  lives.
- **Setters persist.** Every setter updates `config.toml` and the running
  engine.

## How a call is handled

```text
method call
  → update Config, save to disk
  → send a Command to the engine thread
  → emit a change signal
```

Read methods return data from `Config` or from lock-free values published by
the audio callbacks. They never wait on the audio thread.

## Methods

### Groups and streams

| Method | Arguments | Returns | Purpose |
|---|---|---|---|
| `CreateGroup` | name | group id | Create a group |
| `DeleteGroup` | group id | | Delete a group. The Default and Cue groups cannot be deleted. Its streams return to Default. |
| `ListGroups` | | list of (id, name, gain, routed stream ids, protected) | All groups |
| `ListStreams` | | list of (id, name, service, playing, app id) | Detected application streams |
| `GetStreamLevels` | | list of (stream id, peak 0..1) | Per-stream level meters |
| `RouteStreamToGroup` | stream id, group id | | Move a stream into a group. Also remembers the group as the app's preferred group. |
| `UnrouteStream` | stream id | | Move a stream back to the Default group |
| `SetStreamMuted` | stream id, muted | success | Mute the stream itself, via `wpctl` |
| `SetStreamSide` | stream id, `"L"` / `"R"` / other | | Send the whole stream to one channel of its group, or back to stereo. Not persisted. |
| `ListStreamSides` | | list of (stream id, `"L"` or `"R"`) | Streams that are not plain stereo |
| `CreateAppStrip` | stream id | group id | "Own DSP": create a group copied from the stream's current one and move the app's streams into it |
| `SetGroupFeeds` | group id, parent id | | Make a group feed another group's input; `u32::MAX` restores normal output |
| `ListGroupFeeds` | | list of (group id, parent id) | Sub-group relationships |

### Levels and DSP

| Method | Arguments | Returns | Purpose |
|---|---|---|---|
| `SetGain` | group id, value | | Group fader |
| `SetMasterGain` / `GetMasterGain` | value / none | / value | Master fader |
| `SetEqBand` | group id, band index, freq, gain dB, Q, enabled | | One EQ band |
| `SetEqLevel` / `GetEqLevel` | group id, dB / group id | / dB | Whole-curve offset, clamped to ±18 dB |
| `SetHighpass` | group id, freq, Q, enabled | | High-pass filter |
| `SetLowpass` | group id, freq, Q, enabled | | Low-pass filter |
| `SetCompressor` | group id, threshold dB, ratio, attack ms, release ms, makeup dB, enabled | | Compressor |
| `SetGate` | group id, threshold dB, attack ms, release ms, hold ms, enabled | | Gate |
| `SetPan` | group id, pan (−1..1), mono | | Pan and mono/stereo |
| `ListGroupPans` | | list of (group id, pan, mono) | Pan state of every group |
| `GetGroupDetails` | group id | (bands, HPF, LPF, compressor, gate) | Full DSP state of one group |

`GetGroupDetails` returns, in order: a list of (freq, gain dB, Q, enabled)
per band; (freq, Q, enabled) for the high-pass; the same for the low-pass;
(threshold, ratio, attack, release, makeup, enabled) for the compressor;
(threshold, attack, release, hold, enabled) for the gate.

### Plugins

| Method | Arguments | Returns | Purpose |
|---|---|---|---|
| `ListAvailablePlugins` | | list of (type id, name, parameters) | Built-in plugin types |
| `ListGroupPlugins` | group id | list of (index, type, enabled, param names, param values) | A group's chain, in order |
| `AddGroupPlugin` | group id, type id | | Append a plugin |
| `RemoveGroupPlugin` | group id, index | | Remove a slot |
| `SetGroupPluginBypass` | group id, index, bypassed | | Bypass a slot |
| `SetGroupPluginParam` | group id, index, name, value | | Set one parameter |
| `ReorderGroupPlugin` | group id, from, to | | Move a slot |

### Live data

| Method | Returns | Purpose |
|---|---|---|
| `GetLevels` | list of (group id, peak L, peak R, compressor GR dB, gate GR dB) | Output meters and gain reduction |
| `GetSpectrum` (group id) | 24 dB values, 40 Hz to 18 kHz, log-spaced | Input spectrum; empty if the group is not live |
| `GetMetrics` | (CPU %, RAM %, buffer frames, underrun count) | System and engine metrics |
| `GetGraph` | (nodes, edges) | Signal-flow graph: nodes are (id, label, kind, state), edges are (from, to) |
| `RunHealthChecks` | (score, checks) | Doctor report: score is `ok` / `warning` / `error`; checks are (name, severity, message) |

### Outputs and routing

| Method | Arguments | Returns | Purpose |
|---|---|---|---|
| `ListOutputs` | | list of (id, name, description, kind, available, system default, sample rate, channels) | Discovered outputs |
| `GetOutputVolume` | output id | (volume, muted) | The device's own volume |
| `SetOutputVolume` | output id, volume | success | Set it, via `wpctl` |
| `SetOutputMuted` | output id, muted | success | Mute it, via `wpctl` |
| `ListGroupOutputRoutes` | group id | list of (output id, gain, enabled, mode, sync mode, manual delay ms, sync included) | A group's explicit routes |
| `SetGroupOutputRoute` | group id, output id, gain, enabled | | Add or update a route |
| `RemoveGroupOutputRoute` | group id, output id | | Remove a route |
| `SetOutputSynchronization` | group id, output id, sync mode, manual delay ms | | `off`, `automatic` or `manual` |
| `ListMasterOutputRoutes` | | same shape as group routes | Master bus routes |
| `SetMasterOutputRoute` | output id, gain, enabled | | Add or update a Master route |
| `RemoveMasterOutputRoute` | output id | | Remove a Master route |
| `SetMasterOutputSynchronization` | output id, sync mode, manual delay ms | | Sync for a Master route |
| `SetMaxCompensationMs` / `GetMaxCompensationMs` | value / none | / value | Ceiling for delay compensation, clamped to 0–500 ms |
| `SetPreferredDefaultOutput` / `GetPreferredDefaultOutput` | output id / none | / output id | Device for groups with no explicit routes; empty string means none |
| `SetBufferMs` / `GetBufferMs` | value / none | / value | Ring-buffer warm-up cushion, clamped to 10–150 ms |

### Cue

| Method | Arguments | Returns | Purpose |
|---|---|---|---|
| `SetCueOutput` | output id, enabled | | Mark or unmark a cue output |
| `ListCueOutputs` | | list of output ids | Cue outputs |
| `GetCueGroupId` | | group id | The permanent Cue group |
| `SetCueDuckLevel` / `GetCueDuckLevel` | value 0..1 / none | / value | Level of the rest of the mix on cue outputs while cueing |

### Scenes and profiles

| Method | Arguments | Returns | Purpose |
|---|---|---|---|
| `ListScenes` | | list of (id, name, description) | Saved scenes |
| `CaptureCurrentAsScene` | name | scene id | Snapshot the current state as a new scene |
| `UpdateScene` | scene id | success | Re-capture into an existing scene |
| `DuplicateScene` | scene id, new name | scene id | Copy a scene |
| `DeleteScene` | scene id | | Delete a scene |
| `ActivateScene` | scene id | | Apply a scene. Apps not running are applied when they start. |
| `SaveProfileForStream` | stream id, label | success | Save the stream's group and volume against its app identity |

### Rules

| Method | Arguments | Returns | Purpose |
|---|---|---|---|
| `ListRules` | | list of (id, name, enabled, priority, cooldown ms, trigger summary, actions summary) | Rules, summarised |
| `GetRule` | rule id | JSON | One rule in full |
| `SaveRule` | JSON | rule id | Create or replace a rule; validated |
| `CreateAutoRouteRule` | name, template, app id, service, group id | rule id | Create from a fixed template |
| `CreateDeviceConnectedRule` | name, output id, group id, route gain | rule id | Route a group to a device when it connects |
| `SetRuleEnabled` | rule id, enabled | | Enable or disable |
| `DeleteRule` | rule id | | Delete |
| `RuleHistory` | | list of (unix seconds, rule name, trigger, actions, ok, message) | Recent firings, up to 100 |

A rule is structured data: a trigger, optional conditions, a list of
actions, a priority and a cooldown. Rules cannot run arbitrary commands.

| Triggers | Conditions | Actions |
|---|---|---|
| `AppStarted`, `AppStopped`, `DeviceConnected`, `DeviceDisconnected`, `SceneActivated`, `SystemResume` | `OutputAvailable`, `OutputUnavailable` | `RouteTriggeringAppToGroup`, `RouteGroupToOutput`, `SetGroupVolume`, `ActivateScene`, `ApplyProfileForTriggeringApp` |

### Ducking

| Method | Arguments | Returns | Purpose |
|---|---|---|---|
| `ListDuckingRules` | | list of (id, enabled, trigger group, target groups, reduction dB, attack ms, hold ms, release ms, threshold dB) | Ducking rules |
| `CreateDuckingRule` | trigger group, target groups, reduction dB, attack ms, hold ms, release ms, threshold dB | rule id | Create a rule |
| `SetDuckingRuleEnabled` | rule id, enabled | | Enable or disable |
| `DeleteDuckingRule` | rule id | | Delete |

### Recording

| Method | Arguments | Returns | Purpose |
|---|---|---|---|
| `StartRecording` | group id, path, format | recording id | `wav16`, `wav32` or `flac`; `u32::MAX` on a bad format or group |
| `StopRecording` | recording id | | Stop and finalize the file |
| `ListRecordings` | | list of (id, group id, path, elapsed seconds, frames written) | Active recordings |

## Signals

The daemon emits these signals from `/org/mtxpanel/Daemon` when state
changes:

`GroupChanged`, `StreamListChanged`, `GroupOutputRouteChanged`,
`GroupOutputRouteRemoved`, `MasterOutputRouteChanged`,
`MasterOutputRouteRemoved`, `GroupPluginsChanged`, `CueChanged`,
`SceneAdded`, `SceneUpdated`, `SceneRemoved`, `SceneActivated`,
`RuleChanged`, `DuckingRuleChanged`, `RecordingChanged`.

Neither shipped client subscribes to them today. The Tauri UI and the
Quickshell surfaces poll instead. The `MetricsUpdated` signal proposed in
the specification is not emitted; metrics are read with `GetMetrics`.

## Examples

```bash
D="org.mtxpanel.Daemon /org/mtxpanel/Daemon org.mtxpanel.Daemon"

busctl --user call $D CreateGroup s "Music"
busctl --user call $D ListStreams
busctl --user call $D RouteStreamToGroup uu <stream_id> <group_id>
busctl --user call $D SetGain ud <group_id> 0.5
busctl --user call $D SetPan udb <group_id> -0.5 false
busctl --user --json=short call $D GetLevels
```

## Security

The interface is on the session bus and has no access control of its own:
any process in the user's session can call it. Calls that shell out
(`wpctl`) pass arguments as separate process arguments, not through a shell.
Rules are data and cannot execute commands. Recording writes to whatever
path the caller supplies, with the daemon's own permissions.
