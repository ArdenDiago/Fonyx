#!/bin/sh
# Render the Fonyx launch film (~103 s) in 9:16 and 16:9. Run from anywhere:
#
#     sh packaging/branding/video/make.sh                # everything, including filming the real app
#     sh packaging/branding/video/make.sh --no-capture   # reuse footage/ (picture, voice or score changes)
#
# Needs: ffmpeg, curl, `npm ci` in ui/tests (Playwright's Chromium, the bridge) and in site/ (JetBrains
# Mono), pipewire + wireplumber + dbus-run-session for the capture, and a Python with numpy, scipy,
# soundfile and kokoro-onnx. The Kokoro model (~350 MB) and the two fonts are fetched on first run.
#
# Order matters: vo.py src writes what the capture plays into the isolated session; the capture
# films the real UI and records the EQ'd voice from the daemon; vo.py then lays the narration
# (using that recording); music.py follows the takes' logs; the page reads cues.js and timing.js.
set -e
cd "$(dirname "$0")"
PY=${PYTHON:-python3}

mkdir -p models fonts
for f in kokoro-v1.0.onnx voices-v1.0.bin; do
  [ -f models/$f ] || curl -sSfL -o models/$f https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/$f
done
for f in space-grotesk inter; do
  [ -f fonts/$f.woff2 ] || curl -sSfL -o fonts/$f.woff2 "https://cdn.jsdelivr.net/npm/@fontsource-variable/$f@5.1.0/files/$f-latin-wght-normal.woff2"
done

$PY vo.py src
if [ "$1" != "--no-capture" ]; then
  (cd ../../.. && cargo build -p fonyx-daemon -p fonyx-cli &&
    FONYX_TEST_CMD="sh packaging/branding/video/session.sh all" sh ui/tests/run-isolated.sh)
fi
$PY vo.py
$PY music.py
node render.mjs h & node render.mjs v; wait

for m in v h; do
  n=$([ $m = h ] && echo 16x9 || echo 9x16)
  ffmpeg -v error -y -i video-$m.mp4 -i mix.wav -map 0:v -map 1:a -c:v copy \
    -af "loudnorm=I=-14:TP=-1.0:LRA=9" -ar 48000 -c:a aac -b:a 256k -movflags +faststart -shortest fonyx-promo-$n.mp4
  echo "fonyx-promo-$n.mp4"
done
