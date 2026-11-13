// node render.mjs <h|v> [--stills 3,10,...]   frames -> ffmpeg (video only; make.sh adds the sound)
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from '../../../ui/tests/node_modules/playwright/index.mjs';

const dir = path.dirname(fileURLToPath(import.meta.url));
const [mode = 'h', flag, list] = process.argv.slice(2);
const V = mode === 'v', [W, H] = V ? [1080, 1920] : [1920, 1080], FPS = 30;
const cues = fs.readFileSync(`${dir}/cues.js`, 'utf8'), DUR = JSON.parse(cues.slice(cues.indexOf('{'), cues.lastIndexOf('}') + 1)).dur;

const browser = await chromium.launch({ args: ['--allow-file-access-from-files', '--enable-gpu-rasterization', '--ignore-gpu-blocklist'] });
const page = await browser.newPage({ viewport: { width: W, height: H } });
page.on('pageerror', e => { console.error('page error:', e.message); process.exit(1); });
await page.goto(`file://${dir}/promo.html${V ? '?v' : ''}`);
await page.evaluate(() => window.ready);

// footage frames load first, then the frame is drawn and read back
const grab = t => page.evaluate(async t => { await prep(t); render(t); return document.getElementById('c').toDataURL('image/jpeg', 0.93).slice(23); }, t);

if (flag === '--stills') {
  for (const s of list.split(',')) fs.writeFileSync(`${dir}/still-${mode}-${s}.jpg`, Buffer.from(await grab(Number(s)), 'base64'));
} else {
  const ff = spawn('ffmpeg', ['-v', 'error', '-y', '-f', 'image2pipe', '-framerate', String(FPS), '-c:v', 'mjpeg', '-i', '-',
    '-c:v', 'libx264', '-preset', 'slow', '-crf', '17', '-pix_fmt', 'yuv420p', `${dir}/video-${mode}.mp4`], { stdio: ['pipe', 'inherit', 'inherit'] });
  for (let f = 0; f < FPS * DUR; f++) {
    const buf = Buffer.from(await grab(f / FPS), 'base64');
    if (!ff.stdin.write(buf)) await new Promise(r => ff.stdin.once('drain', r));
    if (f % 300 === 0) console.log(mode, `${(f / FPS).toFixed(0)}s`);
  }
  ff.stdin.end();
  await new Promise(r => ff.on('close', r));
}
await browser.close();
