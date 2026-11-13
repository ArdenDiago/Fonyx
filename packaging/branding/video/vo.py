"""Narration: Kokoro TTS, a semitone down with a little weight and room, placed on the film's clock.

    python vo.py src    the sources the capture plays into the isolated session (footage/src-*.wav)
    python vo.py        vo.wav, call.wav and timing.js / timing.json for the page and music.py

The EQ demo's three lines are not processed here: they are played through the real daemon while
the capture drags the real EQ, recorded from the group's output (footage/eq-rec.wav), and that
recording is what the film uses. Without it, the dry lines are used and a warning is printed."""
import hashlib, json, os, sys
import numpy as np
import soundfile as sf
from scipy.signal import resample_poly, sosfilt, correlate, fftconvolve

HERE = os.path.dirname(os.path.abspath(__file__))
M = os.path.join(HERE, 'models') + '/'
CACHE = os.path.join(HERE, 'lines'); os.makedirs(CACHE, exist_ok=True)
FOOT = os.path.join(HERE, 'footage'); os.makedirs(FOOT, exist_ok=True)
_c = open(os.path.join(HERE, 'cues.js')).read(); CUES = json.loads(_c[_c.index('{'): _c.rindex('}') + 1])
SR, DUR = 48000, CUES['dur']
NARRATOR = 'am_fenrir:0.65,am_onyx:0.35'

# id, start (s), text. The eq_* and duck_* lines are played into the app during their takes, so they
# start relative to the moment capture.mjs plays them (EQ_AT, DUCK_AT on the clock).
S, O = CUES['scenes'], CUES['scenes']['open']
LINES = [
    ('tag',   6.3,  "Your audio. Your rules."),
    ('prob',  9.0,  "Music. Videos. Calls. Every application has its own sound, but getting exactly the mix you want shouldn't mean compromising between them."),
    ('meet',  22.6, "Meet Fonyx. A more personal way to control your audio on Linux."),
    ('shape', S['routing'] + 0.4, "Shape your mix, direct your sound, and fine-tune the experience around the way you work and listen."),
    ('more',  S['tour'] + 1.0, "Cue to your headphones. Scenes and rules that switch everything at once. Plugins, recording, and health checks."),
    ('cli',   S['cli'] + 0.4, "And all of it, from the terminal too."),
    ('os1',   O + 0.2, "Fonyx is open source."),
    ('os2',   O + 1.7, "Help us bring it to Windows and macOS."),
]
def played_at(k, planned):   # when capture.mjs actually started the voice in that take, if it has run
    try: return CUES['takes'][k] + next(e['t'] for e in json.load(open(os.path.join(FOOT, k + '.json')))['events'] if e['kind'] == 'voice')
    except (OSError, StopIteration): return CUES['takes'][k] + planned
EQ_AT, DUCK_AT = played_at('eq', 0.2), played_at('duck', 3.0)
EQ_LINES = [('eq1', 0.6, "Thin it out."), ('eq2', 2.4, "Open it up."), ('eq3', 4.2, "And give it weight.")]
DUCK_LINES = [('duck1', 0.5, "When you start talking,"), ('duck2', 2.3, "the music steps aside. Automatically.")]
CALL = "Hey, can you hear me? Yeah, so the build finished, and I think the tests all passed. Can you check the logs for me? I'll wait. Okay, great, then let's ship it tonight."

_k = None
def tts(text, voice, speed):
    key = hashlib.sha1(f'{text}|{voice}|{speed:.2f}'.encode()).hexdigest()[:16]
    path = os.path.join(CACHE, key + '.wav')
    if not os.path.exists(path):
        global _k
        if _k is None:
            from kokoro_onnx import Kokoro
            _k = Kokoro(M + 'kokoro-v1.0.onnx', M + 'voices-v1.0.bin')
        style = sum(w * _k.get_voice_style(n) for n, w in ((p.split(':')[0], float((p + ':1').split(':')[1])) for p in voice.split(',')))
        a, sr = _k.create(text, voice=style, speed=speed, lang='en-us')
        a = resample_poly(a, 2 * 107, 101) if sr == 24000 else resample_poly(a, 107, 101)   # 48 kHz, a semitone down
        nz = np.flatnonzero(np.abs(a) > 0.01)
        sf.write(path, a[max(nz[0] - 480, 0): nz[-1] + 2400].astype(np.float32), SR)
    return sf.read(path)[0].astype(np.float64)

# ---- biquads (RBJ cookbook, as the daemon uses) ----
def peq(f, g, q=1.2):
    A, w = 10 ** (g / 40), 2 * np.pi * f / SR; al = np.sin(w) / (2 * q); c = np.cos(w)
    b = [1 + al * A, -2 * c, 1 - al * A]; a = [1 + al / A, -2 * c, 1 - al / A]
    return np.array(b + a) / a[0]
def hpf(f, q=0.7071):
    w = 2 * np.pi * f / SR; al = np.sin(w) / (2 * q); c = np.cos(w)
    b = [(1 + c) / 2, -(1 + c), (1 + c) / 2]; a = [1 + al, -2 * c, 1 - al]; return np.array(b + a) / a[0]
def lpf(f, q=0.7071):
    w = 2 * np.pi * f / SR; al = np.sin(w) / (2 * q); c = np.cos(w)
    b = [(1 - c) / 2, 1 - c, (1 - c) / 2]; a = [1 + al, -2 * c, 1 - al]; return np.array(b + a) / a[0]
def shelf_low(f, g):
    A, w = 10 ** (g / 40), 2 * np.pi * f / SR; c, s = np.cos(w), np.sin(w); al = s / 2 * np.sqrt(2)
    b = [A * ((A + 1) - (A - 1) * c + 2 * np.sqrt(A) * al), 2 * A * ((A - 1) - (A + 1) * c), A * ((A + 1) - (A - 1) * c - 2 * np.sqrt(A) * al)]
    a = [(A + 1) + (A - 1) * c + 2 * np.sqrt(A) * al, -2 * ((A - 1) + (A + 1) * c), (A + 1) + (A - 1) * c - 2 * np.sqrt(A) * al]
    return np.array(b + a) / a[0]

def dress(x):
    """Weight, presence, glue and a little room: confident, not a movie trailer."""
    x = sosfilt(np.array([hpf(55), shelf_low(140, 3.0), peq(350, -2.0, 1.0), peq(3200, 2.0, 0.9), peq(9000, 1.5, 0.7)]), x)
    env = np.sqrt(np.convolve(x ** 2, np.ones(480) / 480, 'same')) + 1e-6
    x = x * np.convolve(np.minimum(1, (0.12 / env) ** 0.5), np.ones(240) / 240, 'same')
    ir = np.random.default_rng(3).standard_normal(int(SR * 0.7)) * np.exp(-np.arange(int(SR * 0.7)) / (SR * 0.12))
    ir = sosfilt(np.array([lpf(5000), hpf(250)]), ir); ir /= np.sqrt(np.sum(ir ** 2))
    return x + 0.1 * fftconvolve(x, ir)[:len(x)]

def lay(lines, dur, budget_end):
    """Place lines; speed up (to 1.1) any line that would run into the next one."""
    track, out = np.zeros(int(SR * dur)), []
    nxt = [l[1] for l in lines[1:]] + [budget_end]
    for (lid, t0, text), tn in zip(lines, nxt):
        speed = 0.95; a = tts(text, NARRATOR, speed)
        while len(a) / SR > tn - t0 - 0.2 and speed < 1.1:
            speed += 0.05; a = tts(text, NARRATOR, speed)
        i = int(t0 * SR); track[i:i + len(a)] += a[:len(track) - i]
        out.append({'id': lid, 't0': t0, 't1': round(t0 + len(a) / SR, 3), 'text': text})
        print(f'{lid:6s} {t0:6.2f} -> {t0 + len(a) / SR:6.2f}  speed {speed:.2f}  {text[:60]}')
    return track, out

def norm(x, peak): return x * peak / np.max(np.abs(x))

if sys.argv[1:] == ['src']:
    eq, _ = lay(EQ_LINES, 8.0, 8.0)
    eq = np.concatenate([norm(dress(eq), 0.3), np.zeros(SR * 150)])   # outlives the capture session
    sf.write(os.path.join(FOOT, 'src-eqvoice.wav'), eq.astype(np.float32), SR)
    dk, _ = lay(DUCK_LINES, 8.0, 8.0)
    sf.write(os.path.join(FOOT, 'src-duckvoice.wav'), np.concatenate([norm(dress(dk), 0.3), np.zeros(SR * 150)]).astype(np.float32), SR)
    call = tts(CALL, 'af_sarah', 1.0)
    call = np.tile(np.concatenate([call, np.zeros(SR)]), 20)           # long enough never to end mid-capture
    sf.write(os.path.join(FOOT, 'src-call.wav'), norm(call, 0.5).astype(np.float32), SR)
    # something for the Music Player and the Web Browser to play: a plain 120 BPM groove, so the
    # meters and the spectrum in the footage move like music (the film's own score is music.py)
    t = np.arange(SR * 150) / SR; beat = t % 0.5; bar = (t // 2).astype(int) % 4
    root = np.array([45, 41, 36, 43])[bar]; f = 440 * 2 ** ((root - 69) / 12)
    pad = sum(np.sin(2 * np.pi * np.cumsum(f * r) / SR) for r in (2, 2.52, 3, 4.02)) * 0.15
    kick = np.sin(2 * np.pi * (50 + 120 * np.exp(-beat / 0.03)) * beat) * np.exp(-beat / 0.15)
    hat = np.random.default_rng(1).standard_normal(len(t)) * np.exp(-((t + 0.25) % 0.5) / 0.02) * 0.15
    bass = np.sin(2 * np.pi * np.cumsum(f) / SR) * np.exp(-(t % 0.25) / 0.12) * 0.5
    music = pad + kick + sosfilt(np.array([hpf(6000)]), hat) + bass
    sf.write(os.path.join(FOOT, 'src-music.wav'), np.stack([music, music], 1).astype(np.float32) * 0.4 / np.max(np.abs(music)), SR)
    br = sosfilt(np.array([lpf(3000)]), np.roll(music, SR * 7)) + 0.6 * np.roll(np.resize(call, len(music)), SR * 3)
    sf.write(os.path.join(FOOT, 'src-browser.wav'), np.stack([br, br], 1).astype(np.float32) * 0.35 / np.max(np.abs(br)), SR)
    sys.exit()

track, timing = lay(LINES, DUR, S['outro'] + 1.3)
track = dress(track)
# The EQ demo: the daemon's own recording of the Voice group, lined up with the dry source.
src = sf.read(os.path.join(FOOT, 'src-eqvoice.wav'))[0][:SR * 8]
rec_path = os.path.join(FOOT, 'eq-rec.wav')
if os.path.exists(rec_path):
    rec, _ = sf.read(rec_path); rec = rec.mean(1) if rec.ndim > 1 else rec
    lag = int(np.argmax(correlate(rec[:SR * 12], src[:SR * 3], 'valid', method='fft')))   # where the voice starts in the recording
    eq = rec[lag:lag + len(src)]
    print(f'eq voice: daemon recording, voice starts {lag / SR:.3f} s into it')
else:
    print('WARNING: no footage/eq-rec.wav, the EQ demo uses the dry voice'); eq = src
eq = eq * (np.max(np.abs(track)) / np.max(np.abs(src)))             # same level the dry voice would have had
i = int(EQ_AT * SR); track[i:i + len(eq)] += eq
for lid, t0, text in EQ_LINES:
    a = tts(text, NARRATOR, 0.95)
    timing.append({'id': lid, 't0': EQ_AT + t0, 't1': round(EQ_AT + t0 + len(a) / SR, 3), 'text': text})

# The ducking demo: the narrator is the same dry read as everywhere else (Voice has a flat EQ then);
# what the daemon recorded is the Music group, ducked under it. Its level goes to the page for the
# Music lane, and music.py plays the recording itself.
dk, dk_t = lay(DUCK_LINES, 8.0, 8.0)
i = int(DUCK_AT * SR); track[i:i + len(dk)] += dress(dk)
timing += [dict(l, t0=round(DUCK_AT + l['t0'], 3), t1=round(DUCK_AT + l['t1'], 3)) for l in dk_t]
duck_env = []
if os.path.exists(os.path.join(FOOT, 'duck-rec.wav')):
    m, _ = sf.read(os.path.join(FOOT, 'duck-rec.wav')); m = m.mean(1) if m.ndim > 1 else m
    hop = SR // 30
    duck_env = [round(float(20 * np.log10(np.sqrt(np.mean(m[k:k + hop * 3] ** 2)) + 1e-6)), 1) for k in range(0, len(m) - hop * 3, hop)]

call = tts(CALL, 'af_sarah', 1.0)
sf.write('vo.wav', norm(track, 0.5).astype(np.float32), SR)
sf.write('call.wav', call.astype(np.float32), SR)
data = {'lines': timing, 'eq_at': EQ_AT, 'duck_at': DUCK_AT, 'duck_env': duck_env}
json.dump(data, open('timing.json', 'w'), indent=1)
open('timing.js', 'w').write('window.TIMING = ' + json.dumps(data) + ';\n')
