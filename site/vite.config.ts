import { execSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { defineConfig, type Plugin } from 'vite';

const repo = (path: string) => readFileSync(fileURLToPath(new URL(`../${path}`, import.meta.url)), 'utf8');

// The "try it" demo is the real desktop UI, ui/index.html, served at demo/
// with a pretend daemon (public/demo/mock.js) in place of Tauri. It is read
// from the app's source at build time, so the demo cannot drift from the app.
function demo(): Plugin {
  const page = () =>
    repo('ui/index.html')
      .replace(/<link[^>]*fonts\.(googleapis|gstatic)[^>]*>/g, '') // no third-party requests from the site
      .replace('</head>', '<link rel="stylesheet" href="demo.css" /><script src="mock.js"></script></head>'); // after the app's own styles, so demo.css wins

  // Fail the build if the app has a command the pretend daemon cannot answer.
  const check = () => {
    const mock = repo('site/public/demo/mock.js');
    const commands = (repo('ui/src-tauri/src/commands.rs') + repo('ui/src-tauri/src/settings.rs')).matchAll(/#\[tauri::command[^\]]*\]\s*(?:#\[[^\]]*\]\s*)*pub fn (\w+)/g);
    const missing = [...commands].map((m) => m[1]).filter((name) => !mock.includes(`\n    ${name}: (`));
    if (missing.length) throw new Error(`site/public/demo/mock.js is missing: ${missing.join(', ')}`);
  };

  return {
    name: 'sonicforge-demo',
    buildStart: check,
    configureServer(server) {
      server.middlewares.use((req, res, next) => {
        if (req.url !== '/demo/' && req.url !== '/demo/index.html') return next();
        res.setHeader('Content-Type', 'text/html');
        res.end(page());
      });
    },
    generateBundle() {
      this.emitFile({ type: 'asset', fileName: 'demo/index.html', source: page() });
    },
  };
}

// The hero lists the people who built SonicForge, most commits first. The list
// and the photos come from GitHub when the site is built, and are saved in the
// repository (contributors.json, public/contributors/), so visitors' browsers
// never contact GitHub and a build with no network or no access reuses the
// last saved list. The repository is private for now, so asking needs a token:
// GITHUB_TOKEN if set, otherwise the `gh` command's.
const REPO = 'ArdenDiago/SonicForge';
interface Contributor { login: string; commits: number; photo: string }
const here = (path: string) => fileURLToPath(new URL(path, import.meta.url));

function contributors(): Plugin {
  const saved = (): Contributor[] => (existsSync(here('contributors.json')) ? JSON.parse(readFileSync(here('contributors.json'), 'utf8')) : []);

  async function refresh() {
    let token = process.env.GITHUB_TOKEN;
    if (!token) try { token = execSync('gh auth token', { stdio: ['ignore', 'pipe', 'ignore'] }).toString().trim(); } catch { /* no gh: try without */ }
    const headers = { 'User-Agent': 'sonicforge-site', ...(token ? { Authorization: `Bearer ${token}` } : {}) };
    const res = await fetch(`https://api.github.com/repos/${REPO}/contributors?per_page=100`, { headers });
    if (!res.ok) throw new Error(`GitHub answered ${res.status}`);
    const people: { login: string; contributions: number; avatar_url: string }[] = await res.json();
    mkdirSync(here('public/contributors'), { recursive: true });
    const list: Contributor[] = [];
    for (const person of people) { // GitHub already sorts by number of commits
      const photo = await fetch(`${person.avatar_url}&s=96`);
      const ext = photo.headers.get('content-type')?.includes('png') ? 'png' : 'jpg';
      writeFileSync(here(`public/contributors/${person.login}.${ext}`), Buffer.from(await photo.arrayBuffer()));
      list.push({ login: person.login, commits: person.contributions, photo: `contributors/${person.login}.${ext}` });
    }
    writeFileSync(here('contributors.json'), JSON.stringify(list, null, 2) + '\n');
  }

  const html = () => {
    const list = saved();
    if (!list.length) return '';
    const face = (c: Contributor) =>
      `<li><a href="https://github.com/${c.login}" title="${c.login}: ${c.commits} commit${c.commits === 1 ? '' : 's'}"><img src="${c.photo}" width="44" height="44" alt="${c.login}"></a></li>`;
    return `<div class="crew">
            <p>Built by <a href="https://github.com/${REPO}/graphs/contributors">${list.length} contributor${list.length === 1 ? '' : 's'}</a>.</p>
            <ul>${list.slice(0, 10).map(face).join('')}${list.length > 10 ? '<li class="more" title="and more">…</li>' : ''}</ul>
          </div>`;
  };

  return {
    name: 'sonicforge-contributors',
    async buildStart() {
      if (this.meta.watchMode || process.env.NODE_ENV === 'development') return; // only ask GitHub on a real build
      try { await refresh(); } catch (why) { this.warn(`Contributors not refreshed (${(why as Error).message}); using the saved list of ${saved().length}.`); }
    },
    transformIndexHtml: (page) => page.replace('<!-- contributors -->', html()),
  };
}

// Relative base so the built site works from any path (GitHub Pages, a subfolder, file server).
export default defineConfig({
  base: './',
  plugins: [demo(), contributors()],
  // `npm run dev:phone` serves on the network; also accept Tailscale names, which Vite would otherwise refuse.
  server: { allowedHosts: ['.ts.net'] },
  build: {
    rollupOptions: { input: ['index.html', 'privacy.html', 'open-source.html'] },
    chunkSizeWarningLimit: 700, // three.js is one ~590 kB chunk, loaded after the page; that is expected
  },
});
