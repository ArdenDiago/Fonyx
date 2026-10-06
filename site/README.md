# SonicForge website

The public site for SonicForge: one landing page with a 3D mixing desk, plus
`privacy.html` and `open-source.html`. Static files only, no backend.

```bash
npm install
npm run dev       # http://localhost:5173
npm run build     # type-checks, then writes dist/ (host that folder anywhere)
```

- `src/desk.ts` builds the desk in three.js; `src/textures.ts` draws its
  lettering, meter faces, tape and wood onto canvases. There are no 3D model
  or image assets.
- `src/style.css` is shared by all three pages.
- The "try it" demo is the real desktop UI: the build reads `../ui/index.html`
  and serves it at `demo/` with `public/demo/mock.js`, a pretend daemon,
  standing in for Tauri. The build fails if the app gains a command the mock
  does not answer, so add it there too.
- Fonts are bundled from `@fontsource`, so the site makes no third-party
  requests. The privacy page says so; keep it true.
