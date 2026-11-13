// Render an Instagram story page (story.html, app.html) to a PNG of the same
// name. Run from the repository root:
//
//     node packaging/branding/story/render.mjs [app]
//
// Needs `npm ci` in ui/tests (Playwright's Chromium) and in site/ (the fonts).
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from '../../../ui/tests/node_modules/playwright/index.mjs';

const dir = path.dirname(fileURLToPath(import.meta.url));
const name = process.argv[2] ?? 'story';
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1080, height: 1920 } });
await page.goto(`file://${dir}/${name}.html`);
await page.evaluate(() => document.fonts.ready);
await page.screenshot({ path: path.join(dir, `${name}.png`) });
await browser.close();
console.log(`${name}.png 1080x1920`);
