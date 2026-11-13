"""Score and sound design on the film's clock (cues.js), mixed with the narration. Writes mix.wav.

The problem scene's "apps" are heard as apps: the score is the music player, call.wav is a voice
call, and the system volume drag turns them all down together. In the fader demo the score drops
exactly as the real Music fader moved in the footage (footage/faders.json), while the call holds.
UI clicks are placed from the takes' event logs. 120 BPM, A minor (Am F C G)."""
import json
import numpy as np
import soundfile as sf
from scipy.signal import sosfilt, butter

src = open('cues.js').read(); C = json.loads(src[src.index('{'): src.rindex('}') + 1])
S, P, PH, TK = C['scenes'], C['problem'], C['phoenix'], C['takes']
SR, DUR = 48000, C['dur']
N = int(SR * DUR); T = np.arange(N) / SR
BEAT = 60 / C['bpm']; BAR = 4 * BEAT
rng = np.random.default_rng(7)
timing = json.load(open('timing.json'))
vo, _ = sf.read('vo.wav'); call, _ = sf.read('call.wav')
take = lambda k: json.load(open(f'footage/{k}.json'))

def lp(x, f, o=2): return sosfilt(butter(o, f, 'low', fs=SR, output='sos'), x)
def hp(x, f, o=2): return sosfilt(butter(o, f, 'high', fs=SR, output='sos'), x)
def bp(x, a, b, o=2): return sosfilt(butter(o, [a, b], 'band', fs=SR, output='sos'), x)
def ramp(a, b): return np.clip((T - a) / (b - a), 0, 1)
def win(a, b, fi=0.05, fo=0.05): return ramp(a - fi, a) * (1 - ramp(b, b + fo))
def ease(x, a, b): y = np.clip((x - a) / (b - a), 0, 1); return y * y * (3 - 2 * y)
def place(dst, src, t0, g=1.0):
    i = int(t0 * SR)
    if i >= len(dst) or i + len(src) <= 0: return
    s = src[max(0, -i): len(dst) - i] if i >= 0 else src[-i:]
    dst[max(i, 0): max(i, 0) + len(s)] += g * s
hz = lambda n: 440 * 2 ** ((n - 69) / 12)
def osc(freq, harm=8):
    ph = np.cumsum(2 * np.pi * freq / SR)
    return sum(np.sin(k * ph) / k for k in range(1, harm + 1)) * (2 / np.pi)
def smooth(f, ms): k = int(SR * ms / 1000); return np.convolve(f, np.ones(k) / k, 'same')

# ---- harmony: one chord per bar from the first cut ----
CH = [[57, 60, 64], [53, 57, 60], [48, 52, 55], [55, 59, 62]]
ROOT = [45, 41, 36, 43]
bar = np.where(T < S['problem'], 0, ((T - S['problem']) // BAR).astype(int) % 4)
bar[T >= S['outro']] = 0
GROOVE = lambda t: S['fonyx'] <= t < S['open'] or t >= S['outro']

pad = np.zeros(N)
for v in range(3):
    f = smooth(np.array([hz(CH[b][v]) for b in range(4)])[bar], 120)
    for det in (-0.12, 0.0, 0.11): pad += osc(f * 2 ** (det / 12))
pad = lp(pad, 1800) / 9 * (ramp(PH['logo'], PH['logo'] + 1.5) * 0.7 + 0.3 * ramp(S['fonyx'] - 0.6, S['fonyx']))

note = lambda f, d, dec: np.sin(2 * np.pi * f * np.arange(int(SR * d)) / SR) * np.exp(-np.arange(int(SR * d)) / (SR * dec))
bass = np.zeros(N)
for k in range(int((S['end'] - S['problem']) / (BEAT / 2))):
    t0 = S['problem'] + k * BEAT / 2
    if S['open'] <= t0 < S['outro'] and k % 2: continue           # the open-source scene breathes
    f = hz(ROOT[int((t0 - S['problem']) // BAR) % 4] + (12 if k % 2 and GROOVE(t0) else 0))
    n = note(f, BEAT / 2, 0.16); place(bass, np.tanh(2.2 * n) * 0.6 + 0.4 * n, t0, 0.45)
bass = lp(bass, 900)

def kick():
    n = int(SR * 0.45); t = np.arange(n) / SR
    return np.sin(np.cumsum(2 * np.pi * (42 + 110 * np.exp(-t / 0.035)) / SR)) * np.exp(-t / 0.2) + 0.25 * rng.standard_normal(n) * np.exp(-t / 0.004)
def hat(open_=False):
    n = int(SR * (0.25 if open_ else 0.06)); t = np.arange(n) / SR
    return hp(rng.standard_normal(n), 7000) * np.exp(-t / (0.07 if open_ else 0.015))
def clap():
    n = int(SR * 0.3); t = np.arange(n) / SR
    e = sum(np.exp(-np.clip(t - d, 0, None) / 0.008) * (t >= d) for d in (0, 0.011, 0.022)) + np.exp(-t / 0.09) * 0.6
    return bp(rng.standard_normal(n), 900, 5000) * e * 0.5 + np.sin(2 * np.pi * 190 * t) * np.exp(-t / 0.05) * 0.4
drums = np.zeros(N); K, HC, HO, CL = kick(), hat(), hat(True), clap()
for b in range(int((S['end'] - S['problem']) / BEAT)):
    t0 = S['problem'] + b * BEAT
    if t0 < S['fonyx']:                                         # the music player: a plain groove
        place(drums, K, t0, 0.8); place(drums, HC, t0 + BEAT / 2, 0.18)
    elif t0 < S['open'] or t0 >= S['outro']:
        eq_scene = S['eq'] <= t0 < S['duck']
        if not eq_scene or b % 2 == 0: place(drums, K, t0, 0.95)
        place(drums, HO if b % 4 == 3 else HC, t0 + BEAT / 2, 0.22 if not eq_scene else 0.1)
        if b % 2 == 1 and not eq_scene: place(drums, CL, t0, 0.45)
        if S['tour'] <= t0 < S['open']: place(drums, HC, t0 + BEAT / 4, 0.1); place(drums, HC, t0 + 3 * BEAT / 4, 0.1)
    elif b % 4 == 0: place(drums, K, t0, 0.5)                      # open source: a heartbeat
since = np.where(T >= S['problem'], (T - S['problem']) % BEAT, 10)
pump = 1 - 0.4 * np.exp(-since / 0.11) * (T >= S['fonyx'])

arp = np.zeros(N)
for k in range(int((S['end'] - S['fonyx']) / (BEAT / 4))):
    t0 = S['fonyx'] + k * BEAT / 4
    if S['eq'] <= t0 < S['duck'] and k % 2: continue
    ch = CH[int((t0 - S['problem']) // BAR) % 4]; n_ = ch[[0, 1, 2, 1][k % 4]] + 12 + (12 if k % 8 >= 4 else 0)
    d = int(SR * 0.22); t = np.arange(d) / SR
    place(arp, (np.sin(2 * np.pi * hz(n_) * t) + 0.3 * np.sin(4 * np.pi * hz(n_) * t)) * np.exp(-t / 0.07), t0, 0.12)
arp = lp(arp, 4500)

# ---- sound design ----
sfx = np.zeros(N)
def whoosh(t_peak, rise=0.45, fall=0.3, lo=300, hi=6000, g=0.5):
    n = int(SR * (rise + fall)); t = np.arange(n) / SR
    env = np.where(t < rise, (t / rise) ** 2, np.exp(-(t - rise) / (fall / 3)))
    s = rng.standard_normal(n); out = np.zeros(n); B = 512
    for i in range(0, n, B):
        f = lo * (hi / lo) ** min(t[i] / rise, 1)
        out[i:i + B] = bp(s[i:i + B], f * 0.6, min(f * 1.6, SR / 2 - 100), 1)
    place(sfx, out * env, t_peak - rise, g)
def boom(t0, g=1.0, f=45):
    n = int(SR * 2.2); t = np.arange(n) / SR
    place(sfx, np.sin(2 * np.pi * (f + 40 * np.exp(-t / 0.08)) * t) * np.exp(-t / 0.6) + lp(rng.standard_normal(n), 400) * np.exp(-t / 0.15) * 0.6, t0, g)
def bell(t0, n_, g=0.15, dec=1.2):
    d = int(SR * 3); t = np.arange(d) / SR; f = hz(n_)
    place(sfx, (np.sin(2 * np.pi * f * t) + 0.4 * np.sin(2 * np.pi * f * 2.76 * t) * np.exp(-t / 0.3)) * np.exp(-t / dec), t0, g)
def click(t0, g=0.12, f=2600):
    d = int(SR * 0.03); t = np.arange(d) / SR
    place(sfx, bp(rng.standard_normal(d), f * 0.7, f * 1.4) * np.exp(-t / 0.004) + 0.5 * np.sin(2 * np.pi * 900 * t) * np.exp(-t / 0.006), t0, g)
def slide(t0, t1, g=0.04):                                       # a fader's soft friction
    d = int(SR * (t1 - t0)); t = np.arange(d) / SR
    place(sfx, bp(rng.standard_normal(d), 1500, 5000) * np.sin(np.pi * t / (t1 - t0)) ** 0.5, t0, g)
def sonic_logo(t0, g=1.0):
    """The Fonyx signature: a sub thump, a quick rising meter zip, then A-E-A in glassy FM bells."""
    boom(t0, 0.45 * g, 48)
    d = int(SR * 0.22); t = np.arange(d) / SR
    place(sfx, np.sin(2 * np.pi * np.cumsum(300 + 2400 * (t / 0.22) ** 2) / SR) * np.sin(np.pi * t / 0.22) * 0.12, t0 - 0.2, g)
    for i, n_ in enumerate([69, 76, 81]):
        d = int(SR * 2.5); t = np.arange(d) / SR; f = hz(n_)
        fm = np.sin(2 * np.pi * f * t + 1.2 * np.exp(-t / 0.25) * np.sin(2 * np.pi * f * 3.5 * t))
        place(sfx, fm * np.exp(-t / (0.6 + 0.5 * i)) * 0.16, t0 + i * 0.11, g)

# 1. phoenix: rumble, air rising as it emerges, wingbeats, the impact at the centre, crystalline
#    assembly, and the sonic logo on the name
sfx += lp(rng.standard_normal(N), 90) * 0.5 * ramp(0.0, 1.0) * (1 - ramp(PH['center'], PH['center'] + 1.5))
air = bp(rng.standard_normal(N), 600, 7000) * ramp(PH['emerge'], PH['center'] - 0.1) ** 2 * (T < PH['center']) * 0.12
fire = bp(rng.standard_normal(N), 400, 3500) * 0.2 + hp((rng.random(N) < 0.0008) * rng.standard_normal(N) * 3, 1500) * 0.5
sfx += air + fire * ramp(PH['emerge'], PH['emerge'] + 0.5) * (1 - ramp(PH['center'], PH['logo'])) * 0.25
for tw in np.arange(PH['emerge'] + 0.25, PH['center'] - 0.1, 0.38): whoosh(tw, 0.16, 0.2, 200, 1800, 0.3 if tw < PH['near'] else 0.5)
whoosh(PH['near'], 0.6, 0.25, 200, 6000, 0.45)                      # it passes the camera
whoosh(PH['center'], 1.0, 0.15, 150, 9000, 0.35); boom(PH['center'], 1.1, 42)
for i in range(18):                                                  # particles locking into place
    tt = PH['center'] + 0.35 + i * 0.045
    d = int(SR * 0.12); t = np.arange(d) / SR
    place(sfx, np.sin(2 * np.pi * hz(84 + (i * 5) % 19) * t) * np.exp(-t / 0.03), tt, 0.05)
bell(PH['logo'], 88, 0.06, 1.0); bell(PH['pulse'], 93, 0.04, 0.8)
sonic_logo(PH['name'])

# 2. the problem: the apps come in, the music swells over the call, the one slider takes them all down
for at in P['apps']:
    whoosh(at + 0.2, 0.2, 0.2, 800, 4000, 0.08)
bell(P['apps'][3], 88, 0.12, 0.35); bell(P['apps'][3] + 0.12, 93, 0.12, 0.5)   # a notification
dl, d0, d1 = P['dragTo'], *P['drag']
system = 1 - (1 - dl) * ease(T, d0, d1)                              # the one system slider
click(d0 - 0.05, 0.1); slide(d0, d1)
swell = 1 + 0.9 * ease(T, *P['swell'])                                # the music creeps up over the call
phone = lambda x: bp(x, 320, 3400, 3) * 1.4
calltrack = np.zeros(N); place(calltrack, phone(call), P['apps'][2])
video = np.zeros(N)                                                  # the browser's video: a plucked tune
for k, n_ in enumerate([76, 79, 81, 79, 76, 74, 76, 72] * 4):
    t0 = P['apps'][1] + k * BEAT; d = int(SR * 0.4); t = np.arange(d) / SR
    place(video, np.sin(2 * np.pi * hz(n_) * t) * np.exp(-t / 0.15), t0, 0.06)
apps = (calltrack * 0.22 + video) * win(P['apps'][1], P['split'], 0.02, 0.4) * system
whoosh(P['split'] + 0.25, 0.5, 0.2, 300, 7000, 0.35); bell(P['split'] + 0.3, 81, 0.05, 1.0)
riser = hp(rng.standard_normal(N), 800) * ramp(P['split'], S['fonyx']) ** 2 * (T < S['fonyx']) * 0.25
sfx += riser

# 3. Fonyx: the lift, then the demos with the real takes' timing
boom(S['fonyx'], 0.9, 46); bell(S['fonyx'], 76, 0.07, 2.0); bell(S['fonyx'], 81, 0.07, 2.0)
fd = take('faders'); t_off = TK['faders']
mv = [(e['t'] + t_off, e['db']) for e in fd['events'] if e['kind'] == 'music']
db0 = mv[0][1]
music_fader = np.ones(N)
m_t, m_db = np.array([x for x, _ in mv]), np.array([y for _, y in mv])
inside = (T >= m_t[0]) & (T < S['routing'])
music_fader[inside] = 10 ** ((np.interp(T[inside], m_t, m_db) - db0) / 20)
music_fader = np.minimum(music_fader + ease(T, S['routing'] - 0.1, S['routing'] + 0.4) * (T < S['routing'] + 1), 1)
grab = next(e['t'] for e in fd['events'] if e['kind'] == 'grab') + t_off
rel = next(e['t'] for e in fd['events'] if e['kind'] == 'release') + t_off
click(grab, 0.1); slide(grab, rel)
held = np.zeros(N); place(held, phone(call[int(SR * 4):]), S['faders'])   # the call, untouched
apps += held * 0.3 * win(S['faders'], S['routing'], 0.3, 0.3)
rt = take('routing'); t_off = TK['routing']
for e in rt['events']:
    if e['kind'] in ('pick', 'route'): click(e['t'] + t_off - 0.05, 0.12)
    if e['kind'] == 'route': whoosh(e['t'] + t_off + 0.35, 0.35, 0.3, 500, 6000, 0.18); bell(e['t'] + t_off + 0.3, 88, 0.05, 0.6)
    if e['kind'] == 'gain': slide(e['t'] + t_off, e['t'] + t_off + 0.9, 0.03)
eqt = take('eq'); t_off = TK['eq']
for e in eqt['events']:
    if e['kind'] in ('thin', 'open', 'weight'): whoosh(e['t'] + t_off + 0.4, 0.4, 0.15, 300, 4000, 0.1)   # the EQ moving by itself
def keys(t_end, n, rate=12):                                       # typing, ending at t_end
    for j in range(n): click(t_end - (n - j) / rate, 0.05, 3800 + 400 * (j % 3))
gt = take('groups'); t_off = TK['groups']
for e in gt['events']:
    if e['kind'] == 'typed': keys(e['t'] + t_off, 4)
    if e['kind'] in ('group', 'pick', 'route'): click(e['t'] + t_off - 0.03, 0.1)
    if e['kind'] == 'route': bell(e['t'] + t_off + 0.2, 88, 0.05, 0.6)
dt = take('duck'); t_off = TK['duck']
for e in dt['events']:
    if e['kind'] in ('source', 'target', 'rule'): click(e['t'] + t_off - 0.03, 0.1)
    if e['kind'] == 'rule': bell(e['t'] + t_off + 0.1, 84, 0.05, 0.6)
# the duck scene's music is the Music group itself, as the daemon recorded it while it ducked
duck_music = np.zeros(N)
m = sf.read('footage/duck-rec.wav')[0]; m = m.mean(1) if m.ndim > 1 else m
rec_t = next(e['t'] for e in dt['events'] if e['kind'] == 'rec') + t_off
place(duck_music, m * 0.32 / np.sqrt(np.mean(m[:SR * 2] ** 2)), rec_t)   # as loud as the score was
whoosh(S['tour'], 0.5, 0.3, 200, 8000, 0.4); boom(S['tour'], 0.5, 50)
for c in (S['groups'], S['faders'], S['routing'], S['eq'], S['duck'], S['together'], S['cli']): whoosh(c, 0.3, 0.25, 400, 5000, 0.18)

# 4. the tour: every click in the take, and the recording path being typed
ft = take('features'); t_off = TK['features']
for e in ft['events']:
    if e['kind'] in ('cue', 'scenes', 'scene', 'rules', 'mtx', 'plugin', 'recording', 'record', 'doctor', 'recheck'): click(e['t'] + t_off - 0.03, 0.1)
    if e['kind'] == 'record': keys(e['t'] + t_off - 0.5, 20, 40)
# the terminal: keys at the page's typing speed (36 chars/s), see sceneCli in promo.html
cli = json.loads(open('footage/cli.js').read().split('=', 1)[1].strip().rstrip(';'))
at = S['cli'] + 0.35
for r in cli:
    keys(at + len(r['cmd']) / 36, len(r['cmd']), 36); click(at + len(r['cmd']) / 36 + 0.05, 0.08, 1800)
    at += len(r['cmd']) / 36 + 0.15 + 0.35

# 5-6. open source, then the outro and the sonic logo again
whoosh(S['open'], 0.6, 0.4, 200, 6000, 0.3); bell(S['open'] + 0.1, 69, 0.06, 2.5)
sonic_logo(S['outro'] + 0.5, 1.1)
whoosh(S['outro'] + 0.5, 0.8, 0.2, 200, 9000, 0.3)

# ---- arrangement ----
music = pad * pump + bass * pump + arp * pump + drums
music[(T >= S['problem']) & (T < S['fonyx'])] *= 0.9
music += lp(osc(np.full(N, hz(33)), 1), 200) * 0.3 * win(0.2, S['problem'], 1.5, 0.3)   # sub under the intro
level = np.where(T < S['fonyx'], swell * system, 1.0) * music_fader        # the music player's own level
level *= np.where((T >= S['eq'] - 0.2) & (T < S['duck']), 0.35, 1.0)        # room for the EQ'd voice
level *= 1 - ease(T, S['duck'] - 0.3, S['duck']) * (1 - ease(T, S['together'] - 0.1, S['together'] + 0.3))   # the recorded Music group plays instead
level *= np.where((T >= S['open']) & (T < S['outro']), 0.7, 1.0)
music *= level
end_pad = sum(osc(np.full(N, hz(n_)), 6) for n_ in [45, 57, 60, 64, 69])
music += lp(end_pad, 2500) / 8 * ramp(S['outro'] + 0.5, S['outro'] + 0.8) * (1 - ramp(S['end'] - 1.5, S['end']))
music *= 1 - ramp(S['end'] - 1.0, S['end'])

# music steps back under the narrator (and further while it explains the open-source part)
duck = np.zeros(N)
for l in timing['lines']:
    if l['id'].startswith(('eq', 'duck')): continue
    duck = np.maximum(duck, ease(T, l['t0'] - 0.1, l['t0'] + 0.1) * (1 - ease(T, l['t1'], l['t1'] + 0.6)))
depth = np.where((T >= S['open']) & (T < S['outro']), 0.55, np.where((T >= S['problem']) & (T < S['fonyx']), 0.35, 0.45))
music *= 1 - depth * duck

mix = np.stack([music, music], 1)
d1, d2 = int(SR * BEAT * 0.75), int(SR * BEAT * 0.375)
mix[:, 0] += 0.5 * np.concatenate([np.zeros(d1), arp[:N - d1]]) * pump * level
mix[:, 1] += 0.5 * np.concatenate([np.zeros(d2), arp[:N - d2]]) * pump * level
mix += np.stack([apps * 0.9, apps * 1.1], 1)                               # the call a touch right
mix += np.stack([sfx, sfx], 1)
mix += np.stack([duck_music, duck_music], 1) * win(S['duck'], S['together'], 0.3, 0.3)[:, None]
mix *= 0.42 / np.max(np.abs(mix))
mix += np.stack([vo, vo], 1)
mix = np.tanh(mix * 1.1) / np.tanh(1.1)
mix *= np.minimum(1, np.clip((S['end'] + 0.4 - T) / 0.4, 0, 1))[:, None]    # then a short silence
mix *= 0.89 / np.max(np.abs(mix))
sf.write('mix.wav', mix.astype(np.float32), SR)
print('mix.wav', N / SR, 's')
