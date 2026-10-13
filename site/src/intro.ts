// The opening sequence: the name, alone; then a singer steps up to a
// microphone, the voice enters it, the signal runs down the cable into a
// mixer, is mixed, and leaves for a pair of headphones. One camera follows
// the line the whole way. Everything is drawn to a 2D canvas
// from the numbers below; like the desk, it uses no image assets.
//
// It plays each time the landing page loads, never under "reduce motion" (the
// stylesheet hides it), and the stylesheet takes it away on its own if this
// file fails to load, so it cannot trap anyone behind it.

type P = [number, number];

const BONE = '#efe7d5', AMBER = '#f0a63c', PETROL = '#17414a';
// One colour per source in the mixer's scope: music, calls, game, browser.
const SOURCES = [AMBER, '#cf4027', '#3fae5a', '#8fd3dd'];
const STRIPS = ['Music', 'Calls', 'Game', 'Browser', 'Chat', 'Default', 'Master'];
// The clock reads zero when the microphone fills the screen. The name and the
// singer come before that, so the sequence starts at -PRE.
const PRE = 2.6, LENGTH = 7.6; // seconds before zero, and after it until the fade to the page
const SAY: [number, string][] = [[-PRE, ''], [-1.1, 'Sound in.'], [1.5, 'Down the line.'], [3.5, 'Through the mixer.'], [5.8, 'To your ears.']];

const clamp = (x: number) => Math.min(Math.max(x, 0), 1);
const ease = (t: number, from: number, to: number) => { const x = clamp((t - from) / (to - from)); return x * x * (3 - 2 * x); };
const mix = (a: number, b: number, f: number) => a + (b - a) * f;
const cycle = (x: number) => ((x % 1) + 1) % 1; // the clock starts below zero, where a bare % would go negative

// A run of cubic curves, sampled to points the signal can travel along.
function trace(start: P, ...curves: [P, P, P][]): P[] {
  const pts: P[] = [start];
  let from = start;
  for (const [a, b, to] of curves) {
    for (let i = 1; i <= 60; i++) {
      const t = i / 60, u = 1 - t;
      pts.push([0, 1].map((k) => u * u * u * from[k] + 3 * u * u * t * a[k] + 3 * u * t * t * b[k] + t * t * t * to[k]) as P);
    }
    from = to;
  }
  return pts;
}
function at(pts: P[], j: number): P {
  const i = Math.min(Math.floor(j), pts.length - 2), f = j - i;
  return [mix(pts[i][0], pts[i + 1][0], f), mix(pts[i][1], pts[i + 1][1], f)];
}

// The world, left to right: microphone at the origin, mixer, headphones.
const MIC: P = [0, -115];
const MIXER = { x: 1300, y: -330, w: 1000, h: 660, bus: -180 };
const PHONES: P = [3000, 40];
// The line starts at the capsule, runs down inside the microphone, then along the cable.
const LINE_IN = trace(MIC,
  [[0, -20], [0, 100], [0, 194]],
  [[0, 430], [330, 470], [520, 300]],
  [[700, 140], [860, -290], [1060, -230]],
  [[1160, -200], [1200, MIXER.bus], [MIXER.x, MIXER.bus]]);
const LINE_OUT = trace([MIXER.x + MIXER.w, MIXER.bus],
  [[2480, MIXER.bus], [2500, 330], [2680, 330]],
  [[2770, 330], [2810, 290], [2810, 224]]);

function run(root: HTMLElement) {
  const canvas = root.querySelector('canvas')!;
  const say = root.querySelector<HTMLElement>('.intro-say')!;
  const bar = root.querySelector<HTMLElement>('.intro-bar')!;
  const ctx = canvas.getContext('2d')!;
  let W = 0, H = 0, dpr = 1, scale = 1;

  const resize = () => {
    dpr = Math.min(devicePixelRatio, 2);
    W = root.clientWidth; H = root.clientHeight;
    canvas.width = W * dpr; canvas.height = H * dpr;
  };
  resize();
  addEventListener('resize', resize);

  // Lines keep a minimum weight on screen however far the camera pulls back.
  const line = (world: number, px = 1.2) => Math.max(world, px / scale);
  const hardware = (alpha = 0.6) => { ctx.globalCompositeOperation = 'source-over'; ctx.globalAlpha = alpha; ctx.strokeStyle = BONE; ctx.lineWidth = line(1.8); ctx.stroke(); ctx.globalAlpha = 1; };
  const body = () => { ctx.globalAlpha = 0.55; ctx.fillStyle = PETROL; ctx.fill(); hardware(); };
  // Light: the same path three times, wide and faint to narrow and bright, added together.
  const glow = (colour: string, alpha = 1) => {
    ctx.globalCompositeOperation = 'lighter'; ctx.strokeStyle = colour; ctx.lineCap = ctx.lineJoin = 'round';
    for (const [w, a] of [[9, 0.07], [4, 0.2], [1, 1]]) { ctx.lineWidth = line(2.6, 1.6) * w; ctx.globalAlpha = a * alpha; ctx.stroke(); }
    ctx.globalAlpha = 1; ctx.globalCompositeOperation = 'source-over';
  };

  // The lit part of a cable, up to `p` of its length. Just behind the head the
  // line carries a burst of waveform; behind that it settles to a faint ripple.
  function signal(pts: P[], p: number, t: number) {
    if (p <= 0) return;
    const head = p * (pts.length - 1), burst = 1 - ease(p, 0.92, 1);
    ctx.beginPath();
    for (let i = 0; i <= Math.ceil(head); i++) {
      const j = Math.min(i, head), [x, y] = at(pts, j);
      const a = pts[Math.max(Math.floor(j) - 1, 0)], b = pts[Math.min(Math.floor(j) + 1, pts.length - 1)];
      const n = Math.hypot(b[0] - a[0], b[1] - a[1]) || 1, d = (head - j) / 9;
      const w = Math.sin(d * 5 - t * 16) * 16 * d * Math.exp(1 - d) * burst + Math.sin(j * 0.5 - t * 10) * 1.6 * clamp(d);
      ctx.lineTo(x - ((b[1] - a[1]) / n) * w, y + ((b[0] - a[0]) / n) * w);
    }
    glow(AMBER);
    if (p >= 1) return;
    const [x, y] = at(pts, head), halo = ctx.createRadialGradient(x, y, 0, x, y, 46);
    halo.addColorStop(0, 'rgba(240,166,60,.55)'); halo.addColorStop(1, 'rgba(240,166,60,0)');
    ctx.globalCompositeOperation = 'lighter'; ctx.fillStyle = halo; ctx.fillRect(x - 46, y - 46, 92, 92);
    ctx.fillStyle = BONE; ctx.beginPath(); ctx.arc(x, y, line(4.5, 2.5), 0, 7); ctx.fill();
    ctx.globalCompositeOperation = 'source-over';
  }

  const cable = (pts: P[], from = 0) => { ctx.beginPath(); for (let i = from; i < pts.length; i++) ctx.lineTo(pts[i][0], pts[i][1]); hardware(0.35); };

  function microphone(t: number) {
    const [x, y] = MIC, lit = ease(t, -1, -0.4);
    ctx.beginPath(); ctx.moveTo(-44, -26); ctx.lineTo(44, -26); ctx.lineTo(26, 170); ctx.lineTo(-26, 170); ctx.closePath(); body();
    ctx.beginPath(); ctx.roundRect(-18, 170, 36, 24, 4); body();
    ctx.beginPath(); ctx.roundRect(-62, -200, 124, 170, 60); body();
    // The grille, and the capsule glowing behind it once there is sound.
    ctx.save(); ctx.clip();
    ctx.globalCompositeOperation = 'lighter'; ctx.fillStyle = AMBER; ctx.globalAlpha = lit * (0.16 + 0.08 * Math.sin(t * 13)); ctx.fillRect(-70, -210, 140, 190);
    ctx.beginPath();
    for (let d = -340; d <= 340; d += 17) { ctx.moveTo(d - 200, y - 200); ctx.lineTo(d + 200, y + 200); ctx.moveTo(d + 200, y - 200); ctx.lineTo(d - 200, y + 200); }
    hardware(0.22);
    ctx.restore();
    ctx.beginPath(); ctx.roundRect(-67, -50, 134, 24, 6); body();
    // The voice: arcs closing in on the capsule from both sides.
    for (let k = 0; k < 3; k++) {
      const ph = cycle(t * 0.9 + k / 3), r = 300 - ph * 200, fade = ph * (1 - ph) * 3.2 * ease(t, -0.3, 0.3) * (1 - ease(t, 2.2, 3));
      for (const side of [0, Math.PI]) { ctx.beginPath(); ctx.arc(x, y, r, side - 0.55, side + 0.55); hardware(fade); }
    }
  }

  // The singer: a head and shoulders in outline, who steps up to the
  // microphone, sings, and is left behind when the camera closes on the capsule.
  function singer(t: number) {
    const near = ease(t, -1.9, -1.1), there = ease(t, -1.9, -1.5) * (1 - ease(t, -0.5, 0.1)), singing = ease(t, -1.1, -0.8);
    if (there <= 0) return;
    const away = 1 - near, jaw = singing * (6 + 5 * Math.sin(t * 14));
    const skin = () => { ctx.globalAlpha = 0.55 * there; ctx.fillStyle = PETROL; ctx.fill(); hardware(0.6 * there); };
    ctx.save();
    ctx.translate(130 + away * 520, -105 - Math.abs(Math.sin(near * Math.PI * 3)) * 12 * away); // a few steps, each with a small rise
    // Shoulders and body, running off the bottom of the picture.
    ctx.beginPath(); ctx.moveTo(-40, 1100); ctx.lineTo(-40, 340); ctx.bezierCurveTo(-40, 190, 10, 120, 80, 95); ctx.lineTo(150, 95); ctx.bezierCurveTo(230, 120, 280, 190, 280, 340); ctx.lineTo(280, 1100); skin();
    // The head in profile, facing the microphone; the jaw drops with each note.
    ctx.beginPath(); ctx.moveTo(86, 100); ctx.lineTo(84, 52); ctx.quadraticCurveTo(50, 50, 32, 22 + jaw);
    ctx.lineTo(22, 4 + jaw); ctx.lineTo(34, -8 + jaw * 0.4); ctx.lineTo(20, -22); ctx.lineTo(24, -34); ctx.lineTo(4, -44); ctx.lineTo(24, -84);
    ctx.bezierCurveTo(22, -150, 70, -190, 120, -180); ctx.bezierCurveTo(190, -168, 210, -90, 180, -30); ctx.quadraticCurveTo(165, 10, 150, 40); ctx.lineTo(152, 100); skin();
    ctx.beginPath(); ctx.moveTo(42, -68); ctx.quadraticCurveTo(52, -61, 63, -68); hardware(0.6 * there); // an eye, closed
    // The voice, from the mouth to the grille.
    for (let k = 0; k < 3; k++) {
      const ph = cycle(t * 1.4 + k / 3);
      ctx.beginPath(); ctx.arc(26, -10, 14 + ph * 70, Math.PI - 0.6, Math.PI + 0.6); glow(AMBER, singing * there * (1 - ph) * 0.8);
    }
    ctx.restore();
  }

  function mixer(t: number) {
    const { x, y, w, h, bus } = MIXER, live = ease(t, 3.4, 4.2);
    ctx.beginPath(); ctx.roundRect(x, y, w, h, 18); body();
    for (const jack of [x - 12, x + w]) { ctx.beginPath(); ctx.roundRect(jack, bus - 14, 12, 28, 3); body(); }
    // The scope across the top, with its graticule.
    ctx.beginPath(); ctx.roundRect(x + 60, y + 50, w - 120, 200, 8); ctx.globalAlpha = 0.5; ctx.fillStyle = '#0e2a31'; ctx.fill(); hardware(0.4);
    ctx.beginPath();
    for (let gx = x + 140; gx < x + w - 60; gx += 80) { ctx.moveTo(gx, bus - 6); ctx.lineTo(gx, bus + 6); }
    hardware(0.25);

    const pitch = (w - 120) / STRIPS.length;
    STRIPS.forEach((name, i) => {
      const cx = x + 60 + (i + 0.5) * pitch, master = i === STRIPS.length - 1;
      const level = ease(t, 3.5 + i * 0.07, 4.4 + i * 0.07) * (0.62 + 0.2 * Math.sin(t * 1.5 + i * 1.7));
      const peak = live * level * (0.5 + 0.5 * Math.abs(Math.sin(t * 9 + i * 2.1) * Math.sin(t * 5.3 + i)));
      // Knob, with its pointer following the fader.
      ctx.beginPath(); ctx.arc(cx, -20, 19, 0, 7);
      const turn = 2.4 + level * 4.6; ctx.moveTo(cx, -20); ctx.lineTo(cx + Math.cos(turn) * 19, -20 + Math.sin(turn) * 19); hardware(0.5);
      // Fader slot and cap.
      ctx.beginPath(); ctx.moveTo(cx - 12, 40); ctx.lineTo(cx - 12, 270); hardware(0.3);
      ctx.beginPath(); ctx.roundRect(cx - 34, mix(270, 40, level) - 11, 44, 22, 4);
      ctx.globalAlpha = 1; ctx.fillStyle = master ? '#cf4027' : BONE; ctx.fill();
      // Meter.
      ctx.beginPath(); ctx.roundRect(cx + 24, 40, 9, 230, 2); hardware(0.3);
      ctx.globalCompositeOperation = 'lighter'; ctx.fillStyle = peak > 0.62 ? AMBER : '#3fae5a';
      ctx.fillRect(cx + 25.5, 269 - peak * 228, 6, peak * 228); ctx.globalCompositeOperation = 'source-over';
      if (15 * scale > 6.5) { ctx.globalAlpha = 0.7; ctx.fillStyle = BONE; ctx.font = "500 15px 'IBM Plex Sans Variable', sans-serif"; ctx.textAlign = 'center'; ctx.fillText(name, cx, 308); ctx.globalAlpha = 1; }
    });

    // Mixing: four sources enter apart and out of step, and leave as one wave.
    const front = (t - 3.3) / 1.3;
    if (front <= 0) return;
    SOURCES.forEach((colour, i) => {
      ctx.beginPath();
      for (let u = 0; u <= Math.min(front, 1); u += 0.004) {
        const px = x + u * w, edge = ease(u, 0.06, 0.16) * (1 - ease(u, 0.84, 0.94)), m = ease(u, 0.3, 0.72);
        const own = (i - 1.5) * 34 + 20 * Math.sin(px * (0.03 + i * 0.011) - t * (6 + i * 2.3) + i);
        const sum = 50 * Math.sin(px * 0.021 - t * 7) * (0.65 + 0.35 * Math.sin(px * 0.006 - t * 2));
        ctx.lineTo(px, bus + edge * mix(own, sum, m));
      }
      glow(colour, 0.85);
    });
  }

  function headphones(t: number) {
    const [x, y] = PHONES, lit = ease(t, 5.85, 6.3);
    for (const r of [190, 172]) { ctx.beginPath(); ctx.arc(x, y, r, Math.PI, 0); hardware(); }
    for (const side of [-1, 1]) {
      ctx.beginPath(); ctx.roundRect(x + side * 190 - 28, y - 20, 56, 204, 24); body();
      if (lit) glow(AMBER, lit * (0.6 + 0.25 * Math.sin(t * 11)));
      ctx.beginPath(); ctx.roundRect(x + side * 151 - 11, y + 6, 22, 150, 10); body();
      // Sound leaving each cup towards the listener.
      for (let k = 0; k < 3; k++) {
        const ph = cycle(t * 1.1 + k / 3), face = side < 0 ? 0 : Math.PI;
        ctx.beginPath(); ctx.arc(x + side * 140, y + 80, 16 + ph * 96, face - 0.8, face + 0.8); glow(AMBER, lit * (1 - ph) * 0.8);
      }
    }
  }

  function frame(t: number) {
    // The camera: empty ground under the name, across to the singer at the microphone, in on the
    // capsule, then the head of the signal, then the whole mixer, then the headphones.
    const p = ease(t, 0.9, 3.3), q = ease(t, 4.7, 5.9);
    const head = at(LINE_IN, p * (LINE_IN.length - 1));
    const follow = ease(t, 1, 1.8), wide = ease(t, 3, 4.3), out = ease(t, 5.3, 6.5);
    // On a tall screen the name goes above the singer rather than beside, and the camera stands closer.
    const tall = W < H * 0.9, arrive = ease(t, -2.1, -1.2), back = 1 - ease(t, -0.8, 0);
    const stage = tall ? { x: 110, w: 820 } : { x: -260, w: 1700 };
    const cx = mix(mix(mix(mix(MIC[0], head[0], follow), MIXER.x + MIXER.w / 2, wide), PHONES[0], out), mix(stage.x - stage.w * 1.4, stage.x, arrive), back);
    const cy = mix(mix(mix(mix(MIC[1], head[1], follow), 0, wide), PHONES[1], out), -120, back);
    const fw = mix(mix(mix(mix(520, 600, follow), 1280, wide), 760, out) * (1 - 0.04 * ease(t, 4.3, 5.3)) * (1 - 0.08 * ease(t, 6.5, 9)), stage.w, back);
    const fh = mix(mix(mix(mix(520, 600, follow), 860, wide), 680, out), 900, back);
    // The scene keeps clear of the caption below it, with a matching margin above.
    const top = 24, foot = Math.min(H * 0.2, 170);
    scale = Math.min(W / fw, (H - top - foot) / fh);
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    ctx.setTransform(dpr * scale, 0, 0, dpr * scale, dpr * (W / 2 - cx * scale), dpr * ((H + top - foot) / 2 - cy * scale));

    // A measured ground, so the camera's travel can be felt.
    const hw = W / 2 / scale, hh = H / 2 / scale, dot = line(2.2, 1.4);
    ctx.fillStyle = BONE; ctx.globalAlpha = 0.11;
    for (let gx = Math.floor((cx - hw) / 80) * 80; gx < cx + hw; gx += 80)
      for (let gy = Math.floor((cy - hh) / 80) * 80; gy < cy + hh + 80; gy += 80) ctx.fillRect(gx - dot / 2, gy - dot / 2, dot, dot);
    ctx.globalAlpha = 1;

    cable(LINE_IN, 60); cable(LINE_OUT);
    singer(t); microphone(t); mixer(t); headphones(t);
    signal(LINE_IN, p, t); signal(LINE_OUT, q, t);

    // The name, on the screen rather than in the world: alone in the middle, then aside as the singer arrives.
    const named = ease(t, -PRE, -PRE + 0.5) * (1 - ease(t, -0.7, -0.2));
    if (named <= 0) return;
    const aside = ease(t, -2, -1.2), mid = (H + top - foot) / 2;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.globalAlpha = named; ctx.fillStyle = BONE; ctx.textAlign = 'center'; ctx.textBaseline = 'middle';
    ctx.font = `800 ${Math.min(W * 0.2, H * 0.24) * mix(1, 0.6, aside)}px 'Big Shoulders Display Variable', sans-serif`;
    ctx.fillText('FONYX', mix(W / 2, tall ? W / 2 : W * 0.25, aside), mix(mid, tall ? H * 0.15 : mid, aside));
    ctx.globalAlpha = 1; ctx.textBaseline = 'alphabetic';
  }

  // ?intro=4.6 holds the sequence at that second, for adjusting a single moment.
  const hold = Number(new URLSearchParams(location.search).get('intro')) || 0;
  document.fonts.load("800 64px 'Big Shoulders Display Variable'"); // the name is drawn to canvas, which does not ask for its font
  let t = hold || -PRE, last = performance.now(), said = '', over = false;
  function finish() {
    if (over) return;
    over = true;
    root.classList.add('done');
    setTimeout(() => { root.remove(); removeEventListener('resize', resize); }, 800);
  }
  function tick(now: number) {
    if (!root.isConnected) return;
    // Capped, so a tab left in the background picks up where it was rather than jumping to the end.
    // Never negative either: the first frame's timestamp can be earlier than the clock read above.
    if (!hold) t += Math.min(Math.max(now - last, 0) / 1000, 0.05);
    last = now;
    frame(t);
    const words = SAY.filter(([from]) => t >= from).pop()![1];
    if (words !== said) { said = say.textContent = words; say.style.animation = 'none'; void say.offsetWidth; say.style.animation = ''; }
    bar.style.transform = `scaleX(${clamp((t + PRE) / (LENGTH + PRE))})`;
    if (t >= LENGTH) finish();
    requestAnimationFrame(tick);
  }
  requestAnimationFrame(tick);

  root.querySelector('button')!.addEventListener('click', finish);
  addEventListener('keydown', (e) => e.key === 'Escape' && finish());
  // Scrolling means "show me the page": do that instead of holding it back.
  addEventListener('wheel', finish, { once: true, passive: true });
  addEventListener('touchmove', finish, { once: true, passive: true });
}

const root = document.querySelector<HTMLElement>('#intro');
if (root && !matchMedia('(prefers-reduced-motion: reduce)').matches) run(root);
