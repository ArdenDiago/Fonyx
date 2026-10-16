// Render the Snap Store banner from banner.html. Run from the repository root:
//
//     node packaging/branding/make-banner.mjs
//
// Needs `npm ci` in ui/tests (Playwright's Chromium) and in site/ (the fonts).
// Two ratios because the store's own docs disagree on 2:1 and 3:1; the
// listing page's "Show image restrictions" says which one it takes. JPEG,
// because the grain makes a PNG of the 2:1 version exceed the 2 MB limit.
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from '../../ui/tests/node_modules/playwright/index.mjs';

const dir = path.dirname(fileURLToPath(import.meta.url));
const browser = await chromium.launch();
for (const [w, h, name] of [[1920, 960, 'banner.jpg'], [2160, 720, 'banner-3x1.jpg']]) {
  const page = await browser.newPage({ viewport: { width: w, height: h } });
  await page.goto(`file://${dir}/banner.html`);
  await page.addStyleTag({ content: `:root{--w:${w}px;--h:${h}px}` });
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ path: path.join(dir, name), type: 'jpeg', quality: 92 });
  console.log(name, `${w}x${h}`);
}
await browser.close();
