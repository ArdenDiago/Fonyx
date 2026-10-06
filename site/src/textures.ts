// Every surface on the desk is drawn here onto a 2D canvas: silkscreen,
// meter faces, console tape, wood. No image files to ship or license.
import * as THREE from 'three';

const INK = '#17343a';
const ENAMEL = '#ddd2b8';
const SILK = "'Michroma', sans-serif";

function make(w: number, h: number) {
  const c = document.createElement('canvas');
  c.width = w;
  c.height = h;
  return [c, c.getContext('2d')!] as const;
}

function finish(c: HTMLCanvasElement) {
  const t = new THREE.CanvasTexture(c);
  t.colorSpace = THREE.SRGBColorSpace;
  t.anisotropy = 8;
  return t;
}

// Small deterministic generator so the desk looks the same on every visit.
function rng(seed: number) {
  return () => {
    seed = (seed * 16807) % 2147483647;
    return seed / 2147483647;
  };
}

function speckle(g: CanvasRenderingContext2D, w: number, h: number, n: number, alpha: number, seed: number) {
  const r = rng(seed);
  for (let i = 0; i < n; i++) {
    g.fillStyle = `rgba(${r() < 0.5 ? '0,0,0' : '255,255,255'},${r() * alpha})`;
    g.fillRect(r() * w, r() * h, 1 + r() * 2, 1 + r() * 2);
  }
}

export const KNOB_LABELS = ['HPF', 'EQ', 'COMP', 'PAN'];
// Fader scale: label and position along the travel (0 = closed, 1 = top).
const FADER_MARKS: [string, number][] = [
  ['+12', 1], ['+6', 0.875], ['0', 0.75], ['10', 0.55], ['20', 0.38], ['30', 0.24], ['40', 0.12], ['∞', 0],
];

export interface PanelLayout {
  width: number; depth: number; strips: number;
  stripX: (i: number) => number;
  knobZ: number[]; buttonZ: number; faderNear: number; faderFar: number; ledX: number;
}

export function panelTexture(L: PanelLayout) {
  const W = 2048;
  const ppu = W / L.width;
  const H = Math.round(L.depth * ppu);
  const [c, g] = make(W, H);
  const X = (x: number) => (x + L.width / 2) * ppu;
  const Y = (z: number) => (z + L.depth / 2) * ppu;

  g.fillStyle = ENAMEL;
  g.fillRect(0, 0, W, H);
  // The master strip is painted a shade deeper, as on a real desk.
  g.fillStyle = '#cdbf9f';
  g.fillRect(X(L.stripX(L.strips - 1) - 0.5), 0, W, H);
  speckle(g, W, H, 9000, 0.035, 7);

  g.strokeStyle = INK;
  g.fillStyle = INK;
  g.lineCap = 'round';

  for (let i = 0; i < L.strips; i++) {
    const cx = L.stripX(i);
    const master = i === L.strips - 1;

    if (i > 0) {
      g.lineWidth = master ? 5 : 1.5;
      g.globalAlpha = master ? 1 : 0.45;
      g.beginPath();
      g.moveTo(X(cx - 0.5), Y(-L.depth / 2 + 0.12));
      g.lineTo(X(cx - 0.5), Y(L.depth / 2 - 0.12));
      g.stroke();
      g.globalAlpha = 1;
    }

    g.textAlign = 'center';
    g.textBaseline = 'middle';
    g.font = `400 ${0.13 * ppu}px ${SILK}`;
    g.fillText(master ? 'MASTER' : String(i + 1), X(cx), Y(-2.78));

    const labels = master ? ['CUE', 'PHONES'] : KNOB_LABELS;
    labels.forEach((label, k) => {
      const kz = L.knobZ[k];
      g.lineWidth = 2.5;
      for (let t = 0; t <= 10; t++) {
        const a = (-135 + t * 27) * (Math.PI / 180);
        const r0 = 0.31 * ppu;
        const r1 = (t % 5 === 0 ? 0.39 : 0.355) * ppu;
        g.beginPath();
        g.moveTo(X(cx) + Math.sin(a) * r0, Y(kz) - Math.cos(a) * r0);
        g.lineTo(X(cx) + Math.sin(a) * r1, Y(kz) - Math.cos(a) * r1);
        g.stroke();
      }
      g.font = `400 ${0.085 * ppu}px ${SILK}`;
      g.fillText(label, X(cx), Y(kz + 0.36));
    });

    g.font = `400 ${0.062 * ppu}px ${SILK}`;
    g.fillText('MUTE', X(cx - 0.2), Y(L.buttonZ + 0.2));
    g.fillText('CUE', X(cx + 0.2), Y(L.buttonZ + 0.2));

    // Fader slot, its scale, and the well the level LEDs sit in.
    const slotX = cx - 0.08;
    g.fillStyle = '#0b1416';
    g.beginPath();
    g.roundRect(X(slotX) - 0.03 * ppu, Y(L.faderFar - 0.08), 0.06 * ppu, (L.faderNear - L.faderFar + 0.16) * ppu, 0.03 * ppu);
    g.fill();
    g.beginPath();
    g.roundRect(X(cx + L.ledX) - 0.065 * ppu, Y(L.faderFar - 0.06), 0.13 * ppu, (L.faderNear - L.faderFar + 0.12) * ppu, 0.02 * ppu);
    g.fill();

    g.fillStyle = INK;
    g.textAlign = 'right';
    g.font = `400 ${0.058 * ppu}px ${SILK}`;
    for (const [label, v] of FADER_MARKS) {
      const z = L.faderNear + (L.faderFar - L.faderNear) * v;
      g.lineWidth = label === '0' ? 5 : 2;
      g.beginPath();
      g.moveTo(X(slotX - 0.07), Y(z));
      g.lineTo(X(slotX - 0.15), Y(z));
      g.stroke();
      g.fillText(label, X(slotX - 0.18), Y(z));
    }
  }
  return finish(c);
}

export function bridgeTexture(width: number, height: number) {
  const W = 2048;
  const H = Math.round((height / width) * W);
  const [c, g] = make(W, H);
  g.fillStyle = '#20282b';
  g.fillRect(0, 0, W, H);
  // Brushed finish: long faint horizontal strokes.
  const r = rng(11);
  for (let i = 0; i < 900; i++) {
    g.fillStyle = `rgba(255,255,255,${r() * 0.035})`;
    g.fillRect(r() * W, r() * H, 200 + r() * 900, 1);
  }
  g.fillStyle = ENAMEL;
  g.textBaseline = 'alphabetic';
  g.font = `400 ${H * 0.085}px ${SILK}`;
  g.textAlign = 'left';
  g.fillText('SonicForge', W * 0.035, H * 0.93);
  g.font = `400 ${H * 0.045}px ${SILK}`;
  g.textAlign = 'right';
  g.globalAlpha = 0.75;
  g.fillText('PER-APPLICATION MIXER FOR LINUX    MODEL 0.1.0', W * 0.965, H * 0.93);
  return finish(c);
}

// A VU face. The needle pivots below the visible face, at (0.5, 1.1) in face units.
export function vuTexture() {
  const W = 512, H = 384;
  const [c, g] = make(W, H);
  const lamp = g.createRadialGradient(W / 2, H * 0.95, 20, W / 2, H * 0.6, W * 0.75);
  lamp.addColorStop(0, '#fff1c4');
  lamp.addColorStop(0.55, '#f1d48d');
  lamp.addColorStop(1, '#b98a3e');
  g.fillStyle = lamp;
  g.fillRect(0, 0, W, H);

  const px = W / 2, py = H * 1.1, R = H * 0.86;
  const ang = (v: number) => -Math.PI / 2 + (v - 0.5) * 1.68; // v in 0..1 across the sweep
  const zero = 0.72; // where 0 VU sits on the sweep

  g.lineWidth = 5;
  g.strokeStyle = '#1b1b1b';
  g.beginPath();
  g.arc(px, py, R, ang(0.04), ang(zero));
  g.stroke();
  g.strokeStyle = '#b3261a';
  g.lineWidth = 16;
  g.beginPath();
  g.arc(px, py, R - 5, ang(zero), ang(0.97));
  g.stroke();

  const marks: [string, number][] = [
    ['20', 0.04], ['10', 0.2], ['7', 0.32], ['5', 0.43], ['3', 0.54], ['1', 0.66], ['0', zero], ['1', 0.81], ['2', 0.89], ['3', 0.97],
  ];
  g.textAlign = 'center';
  g.textBaseline = 'middle';
  g.font = `600 26px 'Hanken Grotesk Variable', sans-serif`;
  for (const [label, v] of marks) {
    const a = ang(v);
    const red = v > zero;
    g.strokeStyle = g.fillStyle = red ? '#b3261a' : '#1b1b1b';
    g.lineWidth = 4;
    g.beginPath();
    g.moveTo(px + Math.cos(a) * R, py + Math.sin(a) * R);
    g.lineTo(px + Math.cos(a) * (R + 22), py + Math.sin(a) * (R + 22));
    g.stroke();
    g.fillText(label, px + Math.cos(a) * (R + 46), py + Math.sin(a) * (R + 46));
  }
  g.fillStyle = '#1b1b1b';
  g.font = `600 44px 'Fraunces Variable', serif`;
  g.fillText('VU', px, H * 0.62);
  return finish(c);
}

// One long strip of console tape with the group names written on it in marker.
export function tapeTexture(names: string[], width: number, height: number) {
  const W = 4096;
  const H = Math.round((height / width) * W);
  const [c, g] = make(W, H);
  const r = rng(3);
  g.fillStyle = '#f4efe2';
  g.beginPath();
  g.moveTo(8 + r() * 14, 0);
  for (let y = 0; y <= H; y += 10) g.lineTo(6 + r() * 16, y); // torn left end
  for (let y = H; y >= 0; y -= 10) g.lineTo(W - 6 - r() * 16, y); // torn right end
  g.closePath();
  g.fill();
  g.globalCompositeOperation = 'source-atop';
  speckle(g, W, H, 5000, 0.05, 5);
  g.fillStyle = '#1d2c52';
  g.textAlign = 'center';
  g.textBaseline = 'middle';
  const step = W / names.length;
  names.forEach((name, i) => {
    g.save();
    g.translate(step * (i + 0.5) + (r() - 0.5) * 30, H * 0.52 + (r() - 0.5) * 14);
    g.rotate((r() - 0.5) * 0.09);
    g.font = `700 ${H * 0.78}px 'Caveat', cursive`;
    g.globalAlpha = 0.9;
    g.fillText(name, 0, 0);
    g.restore();
  });
  return finish(c);
}

export function woodTexture() {
  const W = 1024, H = 512;
  const [c, g] = make(W, H);
  const base = g.createLinearGradient(0, 0, 0, H);
  base.addColorStop(0, '#6f4526');
  base.addColorStop(0.5, '#5d371d');
  base.addColorStop(1, '#73492a');
  g.fillStyle = base;
  g.fillRect(0, 0, W, H);
  const r = rng(23);
  for (let i = 0; i < 260; i++) {
    const y0 = r() * H, amp = 2 + r() * 9, freq = 0.004 + r() * 0.01, phase = r() * 6;
    g.strokeStyle = r() < 0.75 ? `rgba(34,16,6,${0.05 + r() * 0.22})` : `rgba(190,130,80,${0.04 + r() * 0.1})`;
    g.lineWidth = 0.6 + r() * 2.4;
    g.beginPath();
    for (let x = 0; x <= W; x += 16) g.lineTo(x, y0 + Math.sin(x * freq + phase) * amp);
    g.stroke();
  }
  const t = finish(c);
  t.wrapS = t.wrapT = THREE.RepeatWrapping;
  return t;
}

export function rearTexture(labels: string[], jackX: number[], width: number, height: number) {
  const W = 2048;
  const ppu = W / width;
  const H = Math.round(height * ppu);
  const [c, g] = make(W, H);
  g.fillStyle = '#1b2326';
  g.fillRect(0, 0, W, H);
  speckle(g, W, H, 4000, 0.04, 9);
  g.fillStyle = g.strokeStyle = ENAMEL;
  g.textAlign = 'center';
  g.textBaseline = 'middle';
  g.lineWidth = 2;
  labels.forEach((label, i) => {
    const xl = (jackX[i * 2] + width / 2) * ppu;
    const xr = (jackX[i * 2 + 1] + width / 2) * ppu;
    const mid = (xl + xr) / 2;
    g.font = `400 ${0.1 * ppu}px ${SILK}`;
    g.fillText(label, mid, H * 0.2);
    g.beginPath();
    g.moveTo(xl - 0.12 * ppu, H * 0.31);
    g.lineTo(xr + 0.12 * ppu, H * 0.31);
    g.stroke();
    g.font = `400 ${0.07 * ppu}px ${SILK}`;
    g.fillText('L', xl, H * 0.82);
    g.fillText('R', xr, H * 0.82);
  });
  return finish(c);
}

// Alpha ramp along a tube's length: solid up to `from`, gone by `to`.
export function fadeTexture(from: number, to: number) {
  const [c, g] = make(256, 1);
  const ramp = g.createLinearGradient(0, 0, 256, 0);
  ramp.addColorStop(0, '#fff');
  ramp.addColorStop(from, '#fff');
  ramp.addColorStop(to, '#000');
  g.fillStyle = ramp;
  g.fillRect(0, 0, 256, 1);
  return new THREE.CanvasTexture(c);
}
