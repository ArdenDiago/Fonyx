import './intro'; // the opening sequence starts first, and plays over the desk loading below

const canvas = document.querySelector<HTMLCanvasElement>('#desk')!;
const stage = document.querySelector<HTMLElement>('.stage')!;
const shots = [...stage.querySelectorAll<HTMLElement>('.shot')];
const sceneName = document.querySelector<HTMLElement>('#scene-name')!;
const still = matchMedia('(prefers-reduced-motion: reduce)').matches;
const phone = matchMedia('(max-width: 60rem), (pointer: coarse)').matches;

// The page is readable, and shows a still of the desk, before any of this runs.
// The 3D desk is the heavy part (three.js), so it loads afterwards in its own
// file and fades in over the still. If it cannot start, the still simply stays.
function deskFailed(why: unknown) {
  document.documentElement.classList.remove('desk-ready');
  document.documentElement.classList.add('desk-failed');
  // Open the page with ?debug to read the reason on a phone, where there is no console.
  if (location.search.includes('debug')) {
    document.body.insertAdjacentHTML('beforeend', '<pre style="position:fixed;inset:auto 0 0 0;z-index:99;margin:0;padding:12px;background:#000;color:#fff;font-size:12px;white-space:pre-wrap"></pre>');
    document.body.lastElementChild!.textContent = `3D desk did not start: ${why instanceof Error ? why.stack ?? why.message : why}`;
  }
}

try {
  // The desk's lettering is drawn to canvas, so its fonts have to be in first.
  const fonts = ["700 64px Caveat", "400 32px Michroma", "800 52px 'Big Shoulders Display Variable'", "600 26px 'IBM Plex Sans Variable'"];
  const [{ createDesk }] = await Promise.all([
    import('./desk'),
    Promise.race([Promise.all(fonts.map((f) => document.fonts.load(f))), new Promise((r) => setTimeout(r, 2500))]),
  ]);
  const desk = createDesk(canvas, still, phone);
  desk.onScene = (name) => (sceneName.textContent = name);
  canvas.addEventListener('webglcontextlost', () => { desk.pause(true); deskFailed('the browser took the WebGL context away (usually low memory)'); });

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
} catch (why) {
  deskFailed(why);
}

const copy = document.querySelector<HTMLButtonElement>('#copy-install')!;
copy.addEventListener('click', async () => {
  await navigator.clipboard.writeText(document.querySelector('#install-commands')!.textContent!.replace(/^\$ /gm, ''));
  copy.textContent = 'Copied';
  setTimeout(() => (copy.textContent = 'Copy commands'), 1800);
});

// One button under the app. "Start the demo" reloads the app fresh and brings up its
// songs; the button then reads "Stop the demo", which takes the songs away again.
// The app reports the strip coming and going, so its own Stop button resets this one too.
const demo = document.querySelector<HTMLIFrameElement>('#demo')!;
const demoButton = document.querySelector<HTMLButtonElement>('#demo-toggle')!;
let demoRunning = false;
demoButton.addEventListener('click', () => {
  if (demoRunning) return demo.contentWindow!.postMessage('fonyx-demo:stop', location.origin);
  demo.addEventListener('load', () => demo.contentWindow!.postMessage('fonyx-demo:songs', location.origin), { once: true });
  demo.contentWindow!.location.reload();
});
addEventListener('message', (e) => {
  if (e.origin !== location.origin || !/^fonyx-demo:(on|off)$/.test(e.data)) return;
  demoRunning = e.data.endsWith(':on');
  demoButton.textContent = demoRunning ? 'Stop the demo' : 'Start the demo';
  demoButton.classList.toggle('quiet', !demoRunning);
});
