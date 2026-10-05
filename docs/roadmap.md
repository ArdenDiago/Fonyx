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
- Preferred output device and configurable buffer size
- systemd watchdog, graceful shutdown, atomic config writes
- User install script

## Known unfinished work

These are gaps in what exists, found in the code or its documentation.

### Packaging

- The application icon is a placeholder.
- No version tag or release.

### Engine

- No handling for PipeWire restarting while the daemon runs. The
  specification asks for graceful recovery.
- The underrun metric is the engine's own ring-buffer count, not PipeWire's
  driver xrun count.
- The master fader's ramp speeds up with the number of active groups.
- Structural state shared with audio callbacks is behind a mutex rather
  than a lock-free queue.
- Stream meters need a full stereo port set; mono-only streams have none.
- An app's preferred group is per app identity, so all of a browser's tabs
  share it.
- The buffer-size setting applies only to routes created after the change.

### Clients

- Mute, solo, fader links and fader groups exist only in the Tauri UI's
  local storage.
- No client subscribes to the daemon's change signals; all poll.
- The Quickshell app lacks controls added to the Tauri UI later (EQ level
  line, pan, mono/stereo, left/right isolate).
- The pan knob in the Tauri UI has no keyboard control.
- Per-stream left/right isolate is not saved and resets when the stream
  ends.

### Testing

- No automated tests for profiles, multi-output sync, the Doctor tab, the
  cue bus, pan and mono/stereo, or left/right isolate.
- No automated tests of the engine's PipeWire behaviour.
- No tests of the Quickshell surfaces or of the Tauri backend.
- No CI.
- Not run on any distribution other than Arch Linux.

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
2. Extend the smoke tests to the untested areas.

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
