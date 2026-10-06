import { readFileSync } from 'node:fs';
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

// Relative base so the built site works from any path (GitHub Pages, a subfolder, file server).
export default defineConfig({
  base: './',
  plugins: [demo()],
  build: {
    rollupOptions: { input: ['index.html', 'privacy.html', 'open-source.html'] },
  },
});
