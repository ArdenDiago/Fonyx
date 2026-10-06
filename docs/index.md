# SonicForge Documentation

SonicForge is a per-application audio mixer and router for Linux, built on
PipeWire. The [README](../README.md) is the short introduction; these pages
hold the detail.

## Getting started

- [Development](development.md): build, run, test and debug
- [Architecture](architecture.md): the processes, threads and data flow

## Internals

- [Audio engine](audio-engine.md): PipeWire nodes, routing, the ring buffer,
  outputs
- [DSP](dsp.md): the per-group processing chain
- [D-Bus](dbus.md): the daemon's control interface

## Interfaces

- [Command line](cli.md): the `sonicforge` CLI
- [UI](ui.md): the Tauri desktop application
- [Quickshell](quickshell.md): the Quickshell app, widget and popup

## Distribution

- [Packaging](packaging.md): install script and systemd units

## Engineering

- [Testing](testing.md): test suites and engineering verification
- [Design decisions](decisions.md): what was chosen and why
- [Roadmap](roadmap.md): done, open and possible work

## A note on names

The project is called SonicForge. Its earlier name was MTX Panel. The code
uses the new name in lower case: the binaries are `sonicforge-daemon` and
`sonicforge-ui`, the D-Bus service is `org.sonicforge.Daemon`, the config
lives in `~/.config/sonicforge/` and PipeWire nodes are named
`sonicforge.*`. The one place the old name remains is the EQ tab, which is
still titled "MTX Panel".

Upgrading from an install made under the old name: the daemon moves
`~/.config/mtxpanel/` to the new location on first start, and
`packaging/install-user.sh` removes the old services and binaries.

## Other material in the repository

- [`PROJECT_SPEC.md`](../PROJECT_SPEC.md) is the original build brief. The
  project has grown past it; where the two differ, these pages and the
  source describe what exists.
- Source comments are detailed and are the authority for anything not
  covered here. Comments tagged `ponytail:` mark a deliberate
  simplification and name its upgrade path.
- Some source comments cite design notes under `NewImprovenments/`. Those
  files were removed from the repository after the features were built; the
  references are historical.

## Project links

- Source code and issue tracker: <https://github.com/ArdenDiago/SonicForge>

There is no public website, hosted documentation, release page or status
page yet. When they exist, link them here and in the README's link row.

<!-- Placeholder: project site (planned: mtx.arden.tech, not yet live). -->
