# Roadmap

What is done, what is known to be unfinished, and what might come later.
Nothing here is a commitment or a schedule.

## Completed

The nine stages of the [project specification](../PROJECT_SPEC.md):

| Stage | Status |
|---|---|
| 1. Daemon skeleton and stream discovery | Done |
| 2. Groups and routing | Done |
| 3. Gain with parameter smoothing | Done |
| 4. D-Bus interface | Done |
| 5. EQ | Done |
| 6. Compressor and gate | Done |
| 7. Tauri UI | Done |
| 8. Quickshell | Done |
| 9. Packaging | User install script and systemd units |

Work beyond the specification that is implemented:

- Default group, Cue bus with ducking on cue outputs, Master bus
- Multiple outputs per group with manual and automatic delay compensation
- Stable identities for applications and outputs
- Scenes, per-app profiles, rules engine, ducking
- Plugin chain with four built-in plugins
- Recording to WAV and FLAC
- Doctor health checks and signal-flow graph
- Input spectrum analyser and per-stream level meters
- Safety limiter
- EQ level line, EQ presets
- Pan, mono/stereo, per-stream left/right isolate
- Own DSP per application, sub-groups, fader groups, fader links
- Keyboard control in the Tauri UI
- Preferred output device
- Groups and routes as in-graph PipeWire filter nodes (no ring buffer, no
  added latency)
- systemd watchdog, graceful shutdown, atomic config writes
- Endpoint tests for every command, an isolated test session, and CI on
  pull requests
- User install script

## Known unfinished work

These are gaps in what exists, found in the code or its documentation.

### Packaging

- The Flatpak manifest has never been built with `flatpak-builder`.

### Engine

- No xrun count: the underrun metric (and the Doctor and Home tiles using
  it) stays 0 since the ring buffer went. PipeWire's driver xrun count
  needs a profiler listener.
- The master fader's ramp speeds up with the number of active groups.
- Structural state shared with audio callbacks is behind a mutex rather
  than a lock-free queue.
- Stream meters need a full stereo port set; mono-only streams have none.
- A browser tab's site is known only once its title arrives, so a new tab
  plays briefly in the app's group before moving to its site's group.

### Clients

- No client subscribes to the daemon's change signals; all poll.
- Quickshell's Rules tab has the two rule templates, not the full rule
  editor.
- The pan knob in the Tauri UI has no keyboard control.
- Per-stream left/right isolate is not saved and resets when the stream
  ends.

### Testing

- Tests check stored state and call results, not audio output.
- UI tests do not cover the Outputs tab, cue, pan, fader groups or keyboard
  control.
- No tests of the Quickshell surfaces or of the Tauri backend binary.
- No lint or formatting check in CI.
- Developed on Arch Linux; CI builds and tests on Ubuntu 24.04. No other
  distribution has been tried.

### Not implemented from the specification

- The `MetricsUpdated` signal; metrics are polled with `GetMetrics`.
- Opus as a recording format (named in the recording design, not built).
- Loading external plugins.

## In progress

Nothing is tracked as in progress in the repository.

## Planned

No work is formally planned. The most direct next steps, following from the
gaps above, would be:

1. Tag a first release.
2. Extend the UI tests to the untested areas.
3. Add deployment (release builds) to CI.

## Possible future directions

Ideas that fit the architecture but that nobody has committed to:

- Subscribing to D-Bus signals in the Tauri backend instead of polling.
- Moving mute, solo and fader groups into the daemon so all clients share
  them.
- A lock-free command queue to the audio callbacks.
- A PipeWire profiler listener for real driver xrun counts.
- Remembering per-stream left/right isolate per application.
- A project website and hosted documentation.
- Windows and macOS. The specification names them as long-term targets and
  excludes them from this version; no code for them exists.
