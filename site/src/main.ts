import './fonts';
import { createDesk } from './desk';

const canvas = document.querySelector<HTMLCanvasElement>('#desk')!;
const stage = document.querySelector<HTMLElement>('.stage')!;
const shots = [...stage.querySelectorAll<HTMLElement>('.shot')];
const sceneName = document.querySelector<HTMLElement>('#scene-name')!;
const still = matchMedia('(prefers-reduced-motion: reduce)').matches;

// The desk's lettering is drawn to canvas, so its fonts have to be in first.
const fonts = ["700 64px Caveat", "400 32px Michroma", "600 32px 'Fraunces Variable'", "600 26px 'Hanken Grotesk Variable'"];
await Promise.race([Promise.all(fonts.map((f) => document.fonts.load(f))), new Promise((r) => setTimeout(r, 2500))]);

try {
  const desk = createDesk(canvas, still);
  desk.onScene = (name) => (sceneName.textContent = name);

  // Scroll position -> which camera station we are at, easing through each handover.
  const track = () => {
    const mid = scrollY + innerHeight / 2;
    const centres = shots.map((s) => s.offsetTop + s.offsetHeight / 2);
    let i = centres.findIndex((c) => c > mid) - 1;
    if (i === -2) i = shots.length - 1;
    i = Math.max(i, 0);
    const next = centres[i + 1];
    const f = next ? Math.min(Math.max((mid - centres[i]) / (next - centres[i]), 0), 1) : 0;
    desk.setProgress(i + f * f * f * (f * (f * 6 - 15) + 10));
    desk.cycleScenes(Math.round(i + f) === shots.length - 1);
    desk.pause(stage.getBoundingClientRect().bottom < 0);
  };
  addEventListener('scroll', track, { passive: true });
  addEventListener('resize', track);
  track();
  document.documentElement.classList.add('desk-ready');
} catch {
  // No WebGL: the page still reads top to bottom, with a still of the desk.
  document.documentElement.classList.add('no-webgl');
}

const copy = document.querySelector<HTMLButtonElement>('#copy-install')!;
copy.addEventListener('click', async () => {
  await navigator.clipboard.writeText(document.querySelector('#install-commands')!.textContent!.replace(/^\$ /gm, ''));
  copy.textContent = 'Copied';
  setTimeout(() => (copy.textContent = 'Copy commands'), 1800);
});
