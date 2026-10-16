// The opening sequence: the phoenix from the logo flies in and, on one big
// wingbeat, the name rises letter by letter like a row of meters; then, as
// the bird flies off, a singer steps up to a
// microphone, the voice enters it, the signal runs down the cable into a
// mixer, is mixed, and leaves for a pair of headphones. One camera follows
// the line the whole way. Everything is drawn to a 2D canvas
// from the numbers below; like the desk, it uses no image assets.
//
// It plays each time the landing page loads, never under "reduce motion" (the
// stylesheet hides it), and the stylesheet takes it away on its own if this
// file fails to load, so it cannot trap anyone behind it.

type P = [number, number];

const BONE = '#efe7d5', AMBER = '#f0a63c', PETROL = '#17414a', RED = '#cf4027';
// One colour per source in the mixer's scope: music, calls, game, browser.
const SOURCES = [AMBER, '#cf4027', '#3fae5a', '#8fd3dd'];
const STRIPS = ['Music', 'Calls', 'Game', 'Browser', 'Chat', 'Default', 'Master'];
// The clock reads zero when the microphone fills the screen. The phoenix, the
// name and the singer come before that, so the sequence starts at -PRE.
const PRE = 5, LENGTH = 7.6; // seconds before zero, and after it until the fade to the page
const SAY: [number, string][] = [[-PRE, ''], [-0.15, 'Sound in.'], [1.5, 'Down the line.'], [3.5, 'Through the mixer.'], [5.8, 'To your ears.']];

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

// The Fonyx mark (packaging/branding/logo.svg) as numbers: x, y, w, h,
// colour, and the SVG's rotate(degrees, cx, cy). Only the left wing is listed;
// the right one is drawn mirrored.
type Bar = [number, number, number, number, string, number, number, number];
const WING: Bar[] = [[75, 80, 38, 140, RED, -15, 94, 220], [125, 148, 38, 116, AMBER, -10, 144, 264], [175, 224, 38, 84, BONE, -5, 194, 308]];
const TAIL: Bar[] = [[242, 384, 28, 60, AMBER, -30, 256, 356], [242, 384, 28, 84, RED, 0, 256, 356], [242, 384, 28, 60, AMBER, 30, 256, 356]];
const CREST: Bar[] = [[249, 60, 14, 40, AMBER, -70, 256, 142], [249, 42, 14, 58, RED, -44, 256, 142], [249, 56, 14, 44, AMBER, -18, 256, 142]];

function bar(ctx: CanvasRenderingContext2D, [x, y, w, h, colour, deg, cx, cy]: Bar) {
  ctx.save();
  ctx.translate(cx, cy); ctx.rotate((deg * Math.PI) / 180); ctx.translate(-cx, -cy);
  ctx.beginPath(); ctx.roundRect(x, y, w, h, w / 2); ctx.fillStyle = colour; ctx.fill();
  ctx.restore();
}

// The bird, centred on (x, y) in a box `size` wide. `flap` is radians of wing
// lift (negative is the downstroke); the outer feathers swing further than the
// inner ones, so the wing bends instead of moving as a plank.
function phoenix(ctx: CanvasRenderingContext2D, x: number, y: number, size: number, flap: number, tilt: number) {
  ctx.save();
  ctx.translate(x, y); ctx.rotate(tilt); ctx.scale(size / 512, size / 512); ctx.translate(-256, -256);
  for (const mirrored of [false, true]) {
    ctx.save();
    if (mirrored) { ctx.translate(512, 0); ctx.scale(-1, 1); }
    WING.forEach((b, i) => {
      ctx.save();
      ctx.translate(236, 250); ctx.rotate(flap * (1.3 - i * 0.25)); ctx.translate(-236, -250);
      bar(ctx, b);
      ctx.restore();
    });
    ctx.restore();
  }
  ctx.save();
  ctx.translate(256, 356); ctx.rotate(-tilt * 0.6 + flap * 0.15); ctx.translate(-256, -356); // the tail trails the turn
  TAIL.forEach((b) => bar(ctx, b));
  ctx.restore();
  CREST.forEach((b) => bar(ctx, b));
  bar(ctx, [228, 150, 56, 212, BONE, 0, 0, 0]);
  ctx.beginPath(); ctx.moveTo(284, 128); ctx.lineTo(330, 152); ctx.lineTo(282, 162); ctx.closePath();
  ctx.fillStyle = ctx.strokeStyle = AMBER; ctx.lineWidth = 6; ctx.lineJoin = 'round'; ctx.fill(); ctx.stroke();
  ctx.beginPath(); ctx.arc(256, 142, 38, 0, 7); ctx.fillStyle = BONE; ctx.fill();
  ctx.beginPath(); ctx.arc(270, 135, 6.5, 0, 7); ctx.fillStyle = '#0e2a31'; ctx.fill();
  ctx.restore();
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
    const stage = tall ? { x: 110, w: 820 } : { x: -340, w: 1700 };
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

    // The opening, on the screen rather than in the world: the phoenix flies
    // in and, on its big wingbeat, the name rises; then the bird leaves and
    // the name steps aside for the singer. `s` counts from the very start.
    const s = t + PRE, gone = 1 - ease(t, -0.7, -0.2);
    if (gone <= 0) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.globalAlpha = gone;
    const display = (px: number) => `800 ${px}px 'Big Shoulders Display Variable', sans-serif`;
    ctx.font = display(100);
    const size = Math.min((W * 46) / ctx.measureText('FONYX').width, H * 0.26);
    ctx.font = display(size);
    const cap = ctx.measureText('F').actualBoundingBoxAscent, mid = (H + top - foot) / 2;
    // Bird above name, the pair centred; the bird's drawn part is about 0.8 of its box.
    const S = Math.min(size * 1.3, H * 0.36), block = S * 0.8 + cap;
    const bx = W / 2, by = mid - block / 2 + S * 0.42, base = mid + block / 2;

    // Flight: a curve up from the lower left, slowing into a hover.
    const u = 1 - (1 - clamp(s / 1.35)) ** 3;
    const fly = (k: number) => {
      const P = [[bx - W * 0.55, by + H * 0.8], [bx - W * 0.3, by - H * 0.15], [bx - S * 0.25, by + S * 0.5], [bx, by]];
      return [0, 1].map((d) => (1 - k) ** 3 * P[0][d] + 3 * (1 - k) ** 2 * k * P[1][d] + 3 * (1 - k) * k * k * P[2][d] + k ** 3 * P[3][d]);
    };
    const leave = ease(t, -2.3, -1.2), hover = ease(s, 1.6, 2.1);
    // One running phase, so the wings never jump: quick beats in flight, one
    // slow big beat on arrival (its downstroke is what raises the name), quick again to leave.
    const phase = 3.2 * Math.min(s, 1.3) + 1.4 * Math.max(s - 1.3, 0) + 2.2 * Math.max(t + 2.3, 0);
    const amp = mix(mix(0.5, 0.75, ease(s, 1.1, 1.35)), 0.22, hover) + 0.3 * leave;
    // The upstroke lifts less than the downstroke falls: raised all the way, the wings cross over the head.
    const beat = Math.sin(2 * Math.PI * phase), flap = amp * (beat > 0 ? 0.55 * beat : beat);
    let [x, y] = fly(u);
    y += Math.sin(s * 2.4) * S * 0.025 * hover;
    x += W * 0.45 * leave * leave; y -= H * 0.85 * leave * leave;
    const tilt = 0.45 * (1 - ease(s, 0.5, 1.5)) + 0.4 * leave;
    const wingspan = S * (0.38 + 0.62 * u) * (1 - 0.35 * leave);

    // Embers: small bars shed from the wingtips, falling away like peak marks.
    for (let k = 0; k < 52; k++) {
      const born = 0.04 + k * 0.032, age = s - born;
      if (age < 0 || age > 0.95 || born > 1.7) continue;
      const side = k % 2 ? 1 : -1, [ox, oy] = fly(1 - (1 - clamp(born / 1.35)) ** 3);
      const sz = S * (0.38 + 0.62 * clamp(born / 1.35));
      const ex = ox + side * sz * (0.3 + 0.12 * Math.sin(k * 7.3)) + side * W * 0.04 * age;
      const ey = oy - sz * 0.15 + H * (0.05 * age + 0.3 * age * age);
      ctx.save(); ctx.translate(ex, ey); ctx.rotate(k * 1.7 + age * 3 * side);
      ctx.globalAlpha = gone * (1 - age / 0.95) * 0.9; ctx.fillStyle = [RED, AMBER, BONE][k % 3];
      const len = S * (0.03 + 0.04 * ((k * 37) % 5) / 5);
      ctx.beginPath(); ctx.roundRect(-len * 0.18, -len / 2, len * 0.36, len, len * 0.18); ctx.fill();
      ctx.restore();
    }
    ctx.globalAlpha = gone;

    // The big wingbeat's push: one faint ring leaving the bird.
    const push = ease(s, 1.5, 2.2);
    if (push > 0 && push < 1) {
      ctx.globalCompositeOperation = 'lighter'; ctx.strokeStyle = AMBER; ctx.lineWidth = 2;
      ctx.globalAlpha = gone * 0.35 * (1 - push);
      ctx.beginPath(); ctx.arc(bx, by, S * (0.35 + 1.4 * push), 0, 7); ctx.stroke();
      ctx.globalCompositeOperation = 'source-over'; ctx.globalAlpha = gone;
    }
    if (leave < 1) phoenix(ctx, x, y, wingspan, flap, tilt);

    // The name: each letter is first a meter that shoots up, with its peak
    // tick, then fills into the letter. Then it steps aside for the singer.
    const aside = ease(t, -2, -1.2);
    const nx = mix(W / 2, tall ? W / 2 : W * 0.27, aside), ny = mix(base - cap / 2, tall ? H * 0.15 : mid, aside) + cap / 2;
    const widths = [...'FONYX'].map((ch) => ctx.measureText(ch).width), total = widths.reduce((a, b) => a + b, 0);
    ctx.textAlign = 'left'; ctx.textBaseline = 'alphabetic';
    let lx = nx - total / 2;
    [...'FONYX'].forEach((ch, i) => {
      const at0 = 1.45 + i * 0.07, rise = ease(s, at0, at0 + 0.2), fill = ease(s, at0 + 0.14, at0 + 0.4);
      const w = widths[i], colour = i === 4 ? AMBER : BONE, stem = size * 0.11;
      if (rise > 0 && fill < 1) {
        ctx.globalAlpha = gone * (1 - fill);
        ctx.fillStyle = AMBER;
        ctx.beginPath(); ctx.roundRect(lx + w / 2 - stem / 2, ny - cap * rise, stem, cap * rise, stem / 2); ctx.fill();
        ctx.fillStyle = BONE;
        ctx.fillRect(lx + w / 2 - stem / 2, ny - cap * rise - stem * 0.9 - cap * 0.08 * (1 - rise), stem, stem * 0.32);
      }
      if (fill > 0) {
        ctx.save();
        ctx.beginPath(); ctx.rect(lx - 4, ny - cap * fill - 4, w + 8, cap * fill + size * 0.3); ctx.clip();
        ctx.globalAlpha = gone; ctx.fillStyle = colour;
        ctx.fillText(ch, lx, ny + cap * 0.1 * (1 - fill));
        ctx.restore();
      }
      lx += w;
    });
    ctx.globalAlpha = 1;
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
