import { defineConfig } from 'vite';

// Relative base so the built site works from any path (GitHub Pages, a subfolder, file server).
export default defineConfig({
  base: './',
  build: {
    rollupOptions: { input: ['index.html', 'privacy.html', 'open-source.html'] },
  },
});
