// The film's clock, shared by promo.html (as window.CUES), music.py, vo.py and render.mjs (which
// read the object below as JSON). 120 BPM, so every cut lands on a bar line (2 s). Seconds.
window.CUES = {
  "dur": 96.0,
  "bpm": 120,
  "scenes": { "phoenix": 0, "problem": 8, "fonyx": 22, "groups": 26, "faders": 30, "routing": 36, "eq": 42, "duck": 50, "together": 58, "tour": 60, "cli": 76, "open": 80, "outro": 88, "end": 94.8 },
  "phoenix": { "emerge": 0.8, "near": 1.9, "center": 3.2, "logo": 4.2, "pulse": 4.6, "name": 5.2, "tagline": 6.3 },
  "problem": { "apps": [8.4, 9.0, 9.6, 11.4], "swell": [12.2, 15.0], "drag": [17.2, 19.4], "dragTo": 0.35, "split": 20.6 },
  "takes": { "home": 21.6, "groups": 25.6, "faders": 29.6, "routing": 35.8, "eq": 42.2, "duck": 49.6, "features": 59.6 }
};
