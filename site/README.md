# SonicForge website

The public site for SonicForge: one landing page with a 3D mixing desk, plus
`privacy.html` and `open-source.html`. Static files only, no backend.

```bash
npm install
npm run dev       # http://localhost:5173, this machine only
npm run dev:phone # also on your network: open the "Network" address it prints on your phone
npm run preview:phone  # the built site on your network: the honest test of phone speed
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
- The demo's songs are in `public/demo/music/`, with their licences and sources
  in `CREDITS.md` there. Only add music that is CC0 or public domain.
- Fonts are bundled from `@fontsource`, so the site makes no third-party
  requests. The privacy page says so; keep it true.

The 3D desk loads after the page, in its own file, over a still image
(`public/desk.webp`, `public/desk-phone.webp`, the desk alone on a transparent
ground; `public/poster.jpg` is only the picture used in link previews). If it cannot start, the still
stays. Add `?debug` to the address to see the reason on screen. Regenerate the
stills if the desk's look or the hero camera changes.

The hero's contributor photos are fetched from GitHub by `npm run build` and
saved in `contributors.json` and `public/contributors/`. While the repository
is private that needs `gh` signed in or a `GITHUB_TOKEN`; without either, the
build keeps the saved list.
