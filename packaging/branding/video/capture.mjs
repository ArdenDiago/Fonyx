// Films the real UI (ui/index.html served by ui/tests/bridge.mjs, talking to the live daemon in the
// isolated session that session.sh set up). Every move below is a real pointer event on the real
// controls; the daemon really changes the sound. Each take is written as footage/<take>/NNNN.jpg at
// 30 fps plus footage/<take>.json: when things happened (s from the take's first frame) and where
// the interesting controls were (viewport px; divide by log.zoom for the app's CSS px), so the film can zoom onto them and the score can follow.
//
//     node capture.mjs probe | home | groups | faders | routing | eq | duck | features | cli | all
import { execFileSync, spawn } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from '../../../ui/tests/node_modules/playwright/index.mjs';

const dir = path.dirname(fileURLToPath(import.meta.url));
const out = path.join(dir, 'footage');
const root = path.resolve(dir, '../../..');
const [what = 'probe'] = process.argv.slice(2);
// The app lays out as on a 1600 x 1000 window, drawn at 1.5x. The screencast only ever delivers
// CSS-pixel frames, so the viewport is 2400 x 1500 at 1x with the page zoomed 1.5x instead of a
// device pixel ratio; rects and mouse positions below are all in those 2400 x 1500 pixels.
const VW = 2400, VH = 1500, Z = 1.5, FPS = 30;
const fonyx = (...a) => execFileSync(path.join(root, 'target/debug/fonyx'), ['-q', ...a]);
const sleep = ms => new Promise(r => setTimeout(r, ms));
const ease = x => x * x * (3 - 2 * x);

// A visible cursor (headless Chromium draws none) and a ring on every press.
const CURSOR = `
  addEventListener('DOMContentLoaded', () => {
    document.documentElement.style.zoom = '1.5';
    const c = document.createElement('div');
    c.style.cssText = 'position:fixed;left:0;top:0;z-index:2147483647;pointer-events:none;transform:translate(-100px,-100px);';
    c.innerHTML = '<div id="__ring" style="position:absolute;left:-18px;top:-18px;width:36px;height:36px;border-radius:50%;border:2px solid #83F5FF;opacity:0;transform:scale(.4);transition:transform .35s ease-out,opacity .35s ease-out"></div>'
      + '<svg width="26" height="30" viewBox="0 0 26 30" style="position:absolute;left:-3px;top:-2px;filter:drop-shadow(0 2px 3px rgba(0,0,0,.6))">'
      + '<path d="M3 2 L3 24 L9 18.5 L13.5 28 L17.5 26.2 L13 17 L21 17 Z" fill="#F3F7FA" stroke="#080B0F" stroke-width="1.6" stroke-linejoin="round"/></svg>';
    document.body.appendChild(c);
    // the overlay sits inside the zoomed page, so undo the zoom on its position
    addEventListener('pointermove', e => { const z = parseFloat(document.documentElement.style.zoom) || 1; c.style.transform = 'translate(' + e.clientX / z + 'px,' + e.clientY / z + 'px)'; }, true);
    addEventListener('pointerdown', () => { const r = document.getElementById('__ring'); r.style.transition = 'none'; r.style.opacity = 1; r.style.transform = 'scale(.4)';
      requestAnimationFrame(() => { r.style.transition = ''; r.style.opacity = 0; r.style.transform = 'scale(1.3)'; }); }, true);
    // run-isolated.sh's own fake output is gone but remembered; keep it out of the shots
    new MutationObserver(() => document.querySelectorAll('#output-list > div').forEach(d => { if (d.textContent.includes('Test Output')) d.style.display = 'none'; }))
      .observe(document.body, { childList: true, subtree: true });
  });`;

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: VW, height: VH } });
page.on('pageerror', e => console.error('page error:', e.message));
await page.addInitScript(CURSOR);
await page.goto('http://localhost:4571/');
await page.waitForTimeout(2500);

let pos = [VW * 0.8, VH * 0.75];
const glide = async (x, y, dur) => {
  const [x0, y0] = pos, t0 = Date.now();
  for (;;) {
    const f = ease(Math.min(1, (Date.now() - t0) / (dur * 1000)));
    pos = [x0 + (x - x0) * f, y0 + (y - y0) * f];
    await page.mouse.move(...pos);
    if (f >= 1) break;
    await sleep(12);
  }
};
const tab = name => page.click(`.nav-pill button[data-tab="${name}"]`);
const box = async sel => { const b = await page.locator(sel).first().boundingBox(); return b && [b.x, b.y, b.width, b.height]; };
const center = async sel => { const [x, y, w, h] = await box(sel); return [x + w / 2, y + h / 2]; };
// where a vertical dB fader's thumb sits for a value
const thumbY = async (sel, db) => page.locator(sel).first().evaluate((el, db) => {
  const r = el.getBoundingClientRect(), lo = +el.min, hi = +el.max, inset = 16;
  return [r.x + r.width / 2, r.y + inset + (hi - db) / (hi - lo) * (r.height - 2 * inset)];
}, db);
const eqPoint = (freq, db) => page.evaluate(([f, d]) => {
  const svg = document.getElementById('eq-graph'), p = svg.createSVGPoint(); p.x = freqToX(f); p.y = dbToY(d);
  const q = p.matrixTransform(svg.getScreenCTM()); return [q.x, q.y];
}, [freq, db]);
const bandAt = i => page.locator(`#eq-graph [data-kind="band"][data-index="${i}"]`).first().evaluate(el => {
  const r = el.getBoundingClientRect(); return [r.x + r.width / 2, r.y + r.height / 2];
});

async function take(name, seconds, script) {
  const cdp = await page.context().newCDPSession(page), frames = [];
  cdp.on('Page.screencastFrame', f => { frames.push([f.metadata.timestamp, Buffer.from(f.data, 'base64')]); cdp.send('Page.screencastFrameAck', { sessionId: f.sessionId }).catch(() => {}); });
  await cdp.send('Page.startScreencast', { format: 'jpeg', quality: 92, maxWidth: VW, maxHeight: VH, everyNthFrame: 1 });
  await page.mouse.move(pos[0] + 1, pos[1]);                  // force a first frame
  await sleep(150);
  const t0 = Date.now() / 1000, log = { events: [], rects: {} };
  const at = s => sleep(Math.max(0, (t0 + s - Date.now() / 1000) * 1000));
  const ev = (kind, extra = {}) => log.events.push({ t: +(Date.now() / 1000 - t0).toFixed(3), kind, ...extra });
  await script({ at, ev, log });
  await at(seconds);
  await cdp.send('Page.stopScreencast'); await cdp.detach();
  // constant 30 fps: hold the newest frame at or before each tick
  const d = path.join(out, name); fs.rmSync(d, { recursive: true, force: true }); fs.mkdirSync(d, { recursive: true });
  let j = 0;
  for (let k = 0; k < Math.round(seconds * FPS); k++) {
    while (j + 1 < frames.length && frames[j + 1][0] <= t0 + k / FPS) j++;
    fs.writeFileSync(path.join(d, String(k).padStart(4, '0') + '.jpg'), frames[j][1]);
  }
  log.fps = FPS; log.seconds = seconds; log.size = [VW, VH]; log.zoom = Z; log.frames = frames.length; log.firstFrameLag = +(frames[0][0] - t0).toFixed(3);
  fs.writeFileSync(path.join(out, name + '.json'), JSON.stringify(log, null, 1));
  fs.writeFileSync(path.join(out, name + '.js'), `(window.TAKE ||= {})[${JSON.stringify(name)}] = ${JSON.stringify(log)};\n`);   // promo.html is a file:// page
  console.log(`${name}: ${frames.length} source frames, ${Math.round(seconds * FPS)} written`);
}

// ---- 1. one app's level, and only that app's ----
async function faders() {
  fonyx('fader', 'set', 'Music', '-2dB'); fonyx('fader', 'set', 'Call', '0dB');
  await tab('faders'); await page.waitForTimeout(1200);
  const music = '#fader-rack .fader-col:nth-child(2) input[type="range"]', call = '#fader-rack .fader-col:nth-child(4) input[type="range"]';
  pos = [VW * 0.62, VH * 0.6]; await page.mouse.move(...pos);
  await take('faders', 7.0, async ({ at, ev, log }) => {
    log.rects.rack = await box('#fader-rack .fader-col:nth-child(1)');
    log.rects.music = await box('#fader-rack .fader-col:nth-child(2)');
    log.rects.call = await box('#fader-rack .fader-col:nth-child(4)');
    const [mx, my] = await thumbY(music, -2);
    await at(0.4); await glide(mx, my, 1.0);
    await at(1.5); await page.mouse.down(); ev('grab');
    const [, my2] = await thumbY(music, -20), s0 = Date.now();
    for (;;) {                                                   // pull it down, logging the level as it goes
      const f = ease(Math.min(1, (Date.now() - s0) / 2000));
      pos = [mx, my + (my2 - my) * f]; await page.mouse.move(...pos);
      ev('music', { db: +(await page.locator(music).inputValue()) });
      if (f >= 1) break; await sleep(25);
    }
    await page.mouse.up(); ev('release');
    const [cx, cy] = await thumbY(call, 0);
    await at(4.0); await glide(cx + 40 * Z, cy + 30 * Z, 1.0); ev('call-untouched');
  });
}

// ---- 2. a new route: Music to the TV as well ----
async function routing() {
  try { fonyx('mtx', 'rm', 'Music', 'HDMI TV'); } catch { /* not routed yet */ }
  await tab('outputs'); await page.waitForTimeout(1500);
  const card = await page.evaluate(() => [...document.querySelectorAll('#output-list > div')].findIndex(d => d.textContent.includes('HDMI TV')));
  const hdmi = `#output-list > div:nth-child(${card + 1})`;
  pos = [VW * 0.5, VH * 0.75]; await page.mouse.move(...pos);
  await take('routing', 7.0, async ({ at, ev, log }) => {
    log.rects.hdmi = await box(hdmi);
    log.rects.list = await box('#output-list');
    const [sx, sy] = await center(`${hdmi} select`);
    await at(0.3); await glide(sx, sy, 1.0);
    await at(1.4); await page.mouse.down(); await page.mouse.up();
    await page.selectOption(`${hdmi} select`, { label: 'Music' }); ev('pick');
    const [bx, by] = await center(`${hdmi} button.toolbar-btn`);
    await glide(bx, by, 0.7);
    // the list re-renders every few seconds and resets the select, so pick again right before the click
    await at(2.6); await page.selectOption(`${hdmi} select`, { label: 'Music' }); await page.mouse.down(); await page.mouse.up(); ev('route');
    const musicRow = card => `${card} .route-row:has(.name:text-is("Music"))`;
    await page.waitForSelector(musicRow(hdmi));
    const phones = `#output-list > div:nth-child(${await page.evaluate(() => [...document.querySelectorAll('#output-list > div')].findIndex(d => d.textContent.includes('Studio Headphones'))) + 1})`;
    await at(3.4); log.rects.hdmiAfter = await box(hdmi); log.rects.musicNew = await box(musicRow(hdmi)); log.rects.musicSrc = await box(musicRow(phones));
    const [rx, ry] = await page.locator(`${musicRow(hdmi)} input[type="range"]`).first().evaluate(el => {
      const r = el.getBoundingClientRect(), f = (+el.value - +el.min) / (+el.max - +el.min); return [r.x + 12 + f * (r.width - 24), r.y + r.height / 2]; });
    await glide(rx, ry, 0.8);
    await at(4.4); await page.mouse.down(); ev('gain');
    await glide(rx - 70 * Z, ry, 0.9); await page.mouse.up();
    await at(5.6); await glide(rx + 120 * Z, ry + 90 * Z, 0.9);
  });
}

// ---- 3. the EQ follows the narration on its own: thin it out, open it up, give it weight. No cursor:
// the take sets the real UI's bands through its own code (bandState / renderEqGraph / pushBand, as a
// drag does), so the curve moves on screen and the daemon really shapes the voice it records. ----
const stopRecordings = () => JSON.parse(execFileSync(path.join(root, 'target/debug/fonyx'), ['--json', 'record', 'ls'])).forEach(r => fonyx('record', 'stop', String(r.id)));
const EQ_STATES = { thin: [-18, -12, 0, 4, -12, -18], open: [0, 0, 0, 0, 3, 2], weight: [9, 5, -2, 0, 3, 2] };
const setGains = g => page.evaluate(g => { const bs = bandState[selectedGroupId]; g.forEach((v, i) => { bs[i].gain = v; }); renderEqGraph(); g.forEach((_, i) => pushBand(i)); }, g);
async function morph(to, dur) {
  const from = await page.evaluate(() => bandState[selectedGroupId].map(b => b.gain)), t0 = Date.now();
  for (;;) {
    const f = ease(Math.min(1, (Date.now() - t0) / (dur * 1000)));
    await setGains(from.map((v, i) => Math.round((v + (to[i] - v) * f) * 10) / 10));
    if (f >= 1) break; await sleep(30);
  }
}
async function eq() {
  for (let b = 1; b <= 6; b++) fonyx('dsp', 'eq', 'Voice', String(b), '--gain', '0');
  await tab('mtx'); await page.waitForTimeout(800);
  await page.selectOption('#mtx-group', { label: 'Voice' }); await page.waitForTimeout(1500);
  await page.evaluate(() => { document.querySelector('#__ring').parentElement.style.display = 'none'; });
  const rec = path.join(out, 'eq-rec.wav'); fs.rmSync(rec, { force: true });
  await take('eq', 7.5, async ({ at, ev, log }) => {
    log.rects.graph = await box('#eq-graph');
    fonyx('record', 'start', 'Voice', rec, '--format', 'wav32'); ev('rec');
    await at(0.2);
    spawn('pw-play', ['-P', '{ application.name = "Narrator" node.name = "narrator-eq" media.name = "Narration" }', path.join(out, 'src-eqvoice.wav')], { stdio: 'ignore' }); ev('voice');
    // each move lands just as its line starts (vo.py EQ_LINES: +0.6, +2.4, +4.2 from the voice)
    for (const [name, start] of [['thin', 0.45], ['open', 2.25], ['weight', 4.05]]) { await at(start); ev(name); await morph(EQ_STATES[name], 0.4); }
    await at(7.2); stopRecordings(); ev('rec-stop');
  });
  await page.evaluate(() => { document.querySelector('#__ring').parentElement.style.display = ''; });
}

const click = async () => { await page.mouse.down(); await page.mouse.up(); };   // a real click wherever the cursor is
const goTab = async name => { const [x, y] = await center(`.nav-pill button[data-tab="${name}"]`); await glide(x, y, 0.55); await click(); };

// ---- groups: a new app gets a group of its own ----
async function groups() {
  await tab('inputs'); await page.waitForTimeout(1500);
  pos = [VW * 0.45, VH * 0.75]; await page.mouse.move(...pos);
  const row = '#stream-list .row-item:has-text("Game")';
  await take('groups', 6.0, async ({ at, ev, log }) => {
    log.rects.streams = await box('#stream-list'); log.rects.groups = await box('#group-list');
    const [ix, iy] = await center('#new-group-name');
    await at(0.3); await glide(ix, iy, 0.8); await click();
    await page.keyboard.type('Game', { delay: 80 }); ev('typed');
    const [ax, ay] = await center('#groups-sub-groups .new-group-row button'); await glide(ax, ay, 0.4); await click(); ev('group');
    await page.waitForFunction(() => [...document.querySelectorAll('#stream-list select option')].some(o => o.text === 'Game'));
    const [sx, sy] = await center(`${row} select`); await glide(sx, sy, 0.6); await click();
    await page.selectOption(`${row} select`, { label: 'Game' }); ev('pick');
    const [rx, ry] = await center(`${row} button.primary`); await glide(rx, ry, 0.4);
    // the list re-renders as groups refresh and resets the picker: pick again right before the click
    await page.selectOption(`${row} select`, { label: 'Game' }); await click(); ev('route');
    await page.waitForFunction(() => !document.querySelector('#stream-list')?.textContent.includes('Game'));
    await at(4.8); log.rects.groupsAfter = await box('#group-list');
  });
}

// ---- ducking: a real rule, then the narrator talks while the daemon records the Music group ----
async function duck() {
  for (let b = 1; b <= 6; b++) fonyx('dsp', 'eq', 'Voice', String(b), '--gain', '0');
  await tab('rules'); await page.waitForTimeout(1200);
  await page.evaluate(() => document.getElementById('duck-trigger').closest('.card').scrollIntoView({ block: 'center' }));
  await page.waitForTimeout(600);
  pos = [VW * 0.6, VH * 0.7]; await page.mouse.move(...pos);
  const rec = path.join(out, 'duck-rec.wav'); fs.rmSync(rec, { force: true });
  await take('duck', 9.0, async ({ at, ev, log }) => {
    log.rects.card = await page.evaluate(() => { const r = document.getElementById('duck-trigger').closest('.card').getBoundingClientRect(); return [r.x, r.y, r.width, r.height]; });
    fonyx('record', 'start', 'Music', rec, '--format', 'wav32'); ev('rec');
    const [sx, sy] = await center('#duck-trigger');
    await at(0.3); await glide(sx, sy, 0.7); await click();
    await page.selectOption('#duck-trigger', { label: 'Voice' }); ev('source');
    const target = '#duck-target-list label:has-text("Music") input';
    await page.waitForSelector(target);
    const [tx, ty] = await center(target); await glide(tx, ty, 0.5); await click(); ev('target');
    const [ax, ay] = await center('#duck-submit-btn'); await glide(ax, ay, 0.5); await click(); ev('rule');
    await at(2.3); await glide(ax + 220 * Z, ay + 60 * Z, 0.6);
    // offsets match vo.py's DUCK_LINES (voice spawned 3.0 s into the take)
    await at(3.0); spawn('pw-play', ['-P', '{ application.name = "Narrator" node.name = "narrator-duck" media.name = "Narration" }', path.join(out, 'src-duckvoice.wav')], { stdio: 'ignore' }); ev('voice');
    await at(8.7); stopRecordings(); ev('rec-stop');
  });
}

// ---- the rest of the app, in one continuous take: cue, scenes, rules, plugins, recording, doctor ----
async function features() {
  const gid = name => JSON.parse(execFileSync(path.join(root, 'target/debug/fonyx'), ['--json', 'group', 'ls'])).find(g => g.name === name).id;
  fonyx('scene', 'save', 'Work');
  fonyx('fader', 'set', 'Music', '-14dB'); fonyx('fader', 'set', 'Browser', '-20dB'); fonyx('scene', 'save', 'Night');
  fonyx('scene', 'apply', 'Work');
  for (const [name, app, group] of [['Games go to Game', 'Game', 'Game'], ['Calls go to Call', 'Voice Call', 'Call']]) {
    const rule = { id: 4294967295, name, enabled: true, priority: 1, cooldown_ms: 2000,
      trigger: { AppStarted: { app_id: `appname:${app}`, service: null } }, conditions: [], actions: [{ RouteTriggeringAppToGroup: { group_id: gid(group) } }] };
    execFileSync(path.join(root, 'target/debug/fonyx'), ['-q', 'rule', 'save', '-'], { input: JSON.stringify(rule) });
  }
  const recPath = '/tmp/fonyx-demo.flac';
  await tab('outputs'); await page.waitForTimeout(1500);
  pos = [VW * 0.5, VH * 0.75]; await page.mouse.move(...pos);
  const phones = `#output-list > div:nth-child(${await page.evaluate(() => [...document.querySelectorAll('#output-list > div')].findIndex(d => d.textContent.includes('Studio Headphones'))) + 1})`;
  await take('features', 16.5, async ({ at, ev, log }) => {
    log.rects.cue = await box(phones);
    const [qx, qy] = await center(`${phones} .solo-btn`);
    await at(0.3); await glide(qx, qy, 0.7); await click(); ev('cue');
    await at(2.0); await goTab('scenes'); ev('scenes');
    await page.waitForSelector('#scene-list .output-card'); log.rects.scenes = await box('#scene-list');
    const night = '#scene-list .output-card:has(h2:text-is("Night")) button.primary';
    const [nx, ny] = await center(night); await glide(nx, ny, 0.6); await click(); ev('scene');
    await at(5.0); await goTab('rules'); ev('rules');
    await page.waitForSelector('#rule-list .row-item, #rule-list > *'); log.rects.rules = await box('#rule-list');
    const [lx, ly] = await center('#rule-list'); await glide(lx - 200 * Z, ly, 0.7); ev('rule-hover');
    await at(7.5); await goTab('mtx'); ev('mtx');
    await page.selectOption('#mtx-group', { label: 'Music' }); await page.waitForTimeout(200);
    const [px, py] = await center('#add-plugin-type'); await glide(px, py, 0.5); await click();
    await page.selectOption('#add-plugin-type', { label: 'Deep Bass' });
    const [bx, by] = await center('#add-plugin-type + button'); await glide(bx, by, 0.4); await click(); ev('plugin');
    await page.waitForTimeout(400); log.rects.plugins = await box('#plugin-list');
    await at(10.5); await goTab('recording'); ev('recording');
    await page.selectOption('#rec-group', { label: 'Music' }); await page.selectOption('#rec-format', 'flac');
    const [ix, iy] = await center('#rec-path'); await glide(ix, iy, 0.5); await click();
    await page.keyboard.type(recPath, { delay: 18 });
    const [rx, ry] = await center('#recording .new-group-row button'); await glide(rx, ry, 0.4); await click(); ev('record');
    log.rects.recording = await box('#recording');
    await at(13.4); await goTab('doctor'); ev('doctor');
    const [dx, dy] = await center('#doctor .toolbar-btn'); await glide(dx, dy, 0.6); await click(); ev('recheck');
    log.rects.graph = await box('#signal-graph'); log.rects.checks = await box('#doctor-checks');
    await at(15.4); await glide(dx + 300 * Z, dy + 260 * Z, 0.9);
  });
  stopRecordings(); fs.rmSync(recPath, { force: true });
}

// ---- the same engine from a terminal: real commands, real output ----
async function cli() {
  const bin = path.join(root, 'target/debug/fonyx');
  const runs = [['fader', 'set', 'Music', '-6dB'], ['group', 'ls'], ['duck', 'ls']].map(a => ({ cmd: `fonyx ${a.join(' ')}`, out: execFileSync(bin, a, { encoding: 'utf8' }).trimEnd() }));
  fs.writeFileSync(path.join(out, 'cli.js'), `window.CLI = ${JSON.stringify(runs)};\n`);
  runs.forEach(r => console.log(`$ ${r.cmd}\n${r.out}`));
}

// ---- 0. the app at rest, for the reveal: live meters and spectrum, no cursor ----
async function home() {
  await tab('home'); await page.waitForTimeout(1500);
  await page.evaluate(() => { document.querySelector('#__ring').parentElement.style.display = 'none'; });
  await take('home', 6.0, async () => {});
  await page.evaluate(() => { document.querySelector('#__ring').parentElement.style.display = ''; });
}

if (what === 'probe') {
  for (const t of ['home', 'inputs', 'mtx', 'faders', 'outputs']) {
    await tab(t); await page.waitForTimeout(1200); await page.screenshot({ path: `${out}/probe-${t}.png` });
  }
} else {
  const takes = { home, groups, faders, routing, eq, duck, features, cli };
  for (const k of what === 'all' ? Object.keys(takes) : [what]) await takes[k]();
}
await browser.close();
