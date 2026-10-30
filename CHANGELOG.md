# Changelog

Releases are published by `.github/workflows/appimage.yml` as `vX.Y.Z` when
a change to the app is merged into `main`. This file lists what each one
brings; the newest is at the top.

## Next release (0.1.6)

### Audio engine

- **No added latency.** Each group is now one PipeWire filter node, and each
  output route a small filter node, all joined by real PipeWire links in one
  graph cycle. The ring buffer between a group's capture and playback
  streams is gone, and with it about 85 ms of delay and its own underruns.
  Measured with a tone burst from an app's output to the device: the old
  engine added about 85 ms (128 ms against 43 ms for an app with Fonyx
  bypassed); the new one adds none.
- **Survives a PipeWire restart.** If PipeWire crashes or is restarted, the
  daemon reconnects and rebuilds its nodes instead of sitting disconnected
  with every app bypassing the mixer.

### Mixer

- **Mute, solo, fader links and fader groups are saved by the daemon**, so
  the desktop app, the CLI and Quickshell share them and they survive a
  reinstall of the desktop app. The first start moves your existing ones
  over.
- **Browser tabs remember their site.** Moving a YouTube tab to a group now
  sends future YouTube tabs there, without dragging every other tab of the
  browser along.

### CLI

- `fonyx fader mute|unmute|solo|unsolo|link|unlink <group>`
- `fonyx fader fgroup ls|create|rm|set|add|remove|mute|unmute`

### Quickshell

- Pan, mono/stereo, fader groups and shared mute/solo/link on Faders; EQ
  level on MTX Panel; per-stream meters, left/right, Keep and Own DSP on
  Inputs; device volume and mute, cue outputs, Master routes and cue duck
  on Outputs; rule history; preferred output and both autostart switches.
- Fixed: any negative value (an EQ cut, pan left, a threshold) was refused,
  because busctl read it as a command-line option.

### Removed

- The Settings "Buffer size" control: there is no buffer left for it to
  size. The D-Bus method and CLI setting still exist and do nothing.
