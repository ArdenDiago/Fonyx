# DSP

Each group runs one processing chain in its sink's process callback
(`build_group_capture` in `daemon/src/pw.rs`). The building blocks are in
`daemon/src/dsp/` and `daemon/src/plugins.rs`.

## Chain order

This is the order the code applies, per frame:

![Per-group DSP chain in processing order](images/dsp-chain.svg)

Level meters read the signal after the clamp.

> The original specification and some source comments describe the order as
> gain → EQ → HPF/LPF. The implementation runs the filters before the EQ
> bands. For linear filters in series the result is the same.

## Parameter smoothing

`dsp::smoothed::Smoothed` holds a target and a current value as atomics. The
control thread writes the target; the audio thread advances the current
value one sample at a time with a one-pole ramp:

```text
current += (target − current) × coeff
```

`coeff` is derived from a ramp time, defined as the time to settle within
about 1% of a new target. The engine uses 15 ms (`PARAM_RAMP_MS`). The value
snaps to the target once the remaining difference is negligible relative to
the target, which avoids chasing an asymptote into denormals.

Gain, pan, stereo width, every EQ band's frequency, gain and Q, both
filters' frequency and Q, and all compressor and gate parameters are
smoothed this way. Enable switches are plain atomic booleans.

## Gain

The per-sample gain is the product of four smoothed values:

- the group's own fader
- the master fader, shared by all groups
- the ducking multiplier set by the ducking engine
- the EQ level offset (`eq_level_db`, −18 to +18 dB) converted to linear

## Filters and EQ

`dsp::biquad::Biquad` is a Direct Form I biquad with coefficients from the
RBJ Audio EQ Cookbook. It provides peaking, low-shelf, high-pass and
low-pass responses.

- A new group has six peaking bands at 100 Hz, 300 Hz, 1 kHz, 3 kHz, 8 kHz
  and 12 kHz, all enabled at 0 dB.
- The high-pass and low-pass filters default to 80 Hz with Q = 1/√2 and are
  off until enabled.
- Each channel has its own filter state.

Coefficients are rebuilt **once per buffer** from the current smoothed
values, not per sample, because the trigonometry is comparatively
expensive. Rebuilding changes only the coefficients and never the filter's
history, so a parameter change does not click.

## Compressor and gate

`dsp::dynamics::DynamicsProcessor` uses a feed-forward gain computer: the
target gain is computed from the instantaneous level, and the gain itself is
smoothed toward that target with attack and release one-pole coefficients.

The detector input is the larger of the two channels' absolute values for
the frame, and the resulting gain is applied to both channels, so dynamics
never shift the stereo image.

**Compressor.** With level and threshold in dB:

```text
if enabled and level > threshold:
    target = makeup − (level − threshold) × (1 − 1/ratio)
else:
    target = makeup
```

Makeup gain is applied whenever it is non-zero, including while the
compressor is disabled.

**Gate.** When the level is above the threshold the gate is open (0 dB) and
a hold timer is reloaded. Below the threshold it stays open until the hold
time runs out, then closes toward a floor of −80 dB. Opening uses the attack
time; closing uses the release time.

Attack and release coefficients are computed once per buffer:

```text
coeff = 1 − exp(−1 / (time_ms × 0.001 × sample_rate))
```

| Parameter | Compressor default | Gate default |
|---|---|---|
| Threshold | −18 dB | −50 dB |
| Ratio | 2:1 | n/a |
| Attack | 10 ms | 2 ms |
| Release | 100 ms | 150 ms |
| Hold | n/a | 50 ms |
| Makeup | 0 dB | n/a |
| Enabled | no | no |

Compressor and gate gain reduction are published every buffer for the UI's
meters.

## Plugin chain

Plugins implement the in-process `Plugin` trait and run once per frame on a
stereo pair. A group's chain is an ordered list of slots, each with a type,
an enable flag and parameter values. Slots can be added, removed, bypassed,
reordered and have their parameters set over D-Bus. The chain is rebuilt
whenever the list changes.

Built-in plugin types:

| Type id | Name | What it does |
|---|---|---|
| `soft_clipper` | Soft Clipper | `tanh`-shaped saturation, per sample and per channel |
| `stereo_widener` | Stereo Widener | Mid/side width scaling |
| `deep_bass` | Deep Bass | Low-shelf boost |
| `mids` | Mids | Peaking boost or cut at a chosen mid frequency |

Only built-in plugins exist. There is no loading of external plugins
(LV2, VST, or shared objects).

## Mono and pan

After the plugins, each frame goes through `pan_frame`:

```text
mid   = (L + R) / 2
L'    = (mid + (L − mid) × width) × min(1, 1 − pan)
R'    = (mid + (R − mid) × width) × min(1, 1 + pan)
```

`width` is 1 for stereo and 0 for mono, smoothed so switching does not
click. `pan` runs from −1 (left) to +1 (right). It is a balance control: at
center both channels pass at unity, and turning it only attenuates the
opposite side.

## Safety limiter

`dsp::dynamics::Limiter` is always on and has no user controls. It exists
because nothing earlier in the chain is required to keep the signal inside
full scale: EQ boosts, makeup gain and plugins can add up.

- Ceiling: −0.3 dBFS
- Attack: 1 ms, release: 50 ms
- Detector: the larger channel's peak; one gain for both channels

```text
target = min(0, ceiling − level)        (dB)
```

It has no lookahead, so it cannot fully catch an instantaneous step. The
hard clamp after it is the backstop for that case. The limiter is separate
from the optional Soft Clipper plugin, which is a deliberate effect.

## Spectrum analysis

`dsp::spectrum::SpectrumAnalyzer` analyses the group's **input**, left
channel only, before any processing. It runs a 1024-point FFT (`rustfft`)
and reduces it to 24 log-spaced bins from 40 Hz to 18 kHz, in dB with a
−60 dB floor. The FFT is recomputed every fourth callback. All buffers are
allocated when the group is created.

## Per-route processing

After the chain, each output route applies its own delay line, gain and
enable switch in its playback callback. See
[Audio engine](audio-engine.md#delay-compensation).

## Real-time rules

The chain follows these rules, taken from the project specification:

- No allocation in the callback. Filter state, ring buffers, delay lines and
  FFT buffers are allocated when a group or route is created.
- No file I/O or logging in the callback. Recording pushes samples into a
  ring; a separate thread writes the file.
- Per-sample parameters are read from atomics.

One deviation is documented: a small number of structural lists are behind a
mutex taken once per buffer. See
[Design decisions](decisions.md#locks-on-structural-state).

## Tests

The DSP modules have unit tests for smoothing convergence, biquad frequency
response, compressor and gate behaviour, limiter ceiling, delay-line
interpolation, spectrum output, plugin behaviour and the pan law. See
[Testing](testing.md).
