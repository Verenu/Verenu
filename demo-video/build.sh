#!/usr/bin/env bash
# Rebuilds the 60-second Verenu demo video from this checkout.
#
#   demo-video/build.sh [work-dir]
#
# Everything large (captures, music, the MP4) goes to the work directory,
# outside Git. Default: ~/.cache/verenu-demo-video/<timestamp>.
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
work="${1:-$HOME/.cache/verenu-demo-video/$(date +%Y%m%d-%H%M%S)}"
mkdir -p "$work/tmp"
export TMPDIR="$work/tmp" # this job only; keeps Chrome profiles off a shared /tmp

cd "$repo"
[ -d node_modules ] || npm ci --no-audit --no-fund

# Private Vite server for the browser preview UI, on a free port.
port="$(node -e "const s=require('net').createServer().listen(0,'127.0.0.1',()=>{console.log(s.address().port);s.close()})")"
npx vite --host 127.0.0.1 --port "$port" --strictPort >"$work/vite.log" 2>&1 &
vite_pid=$!
trap 'kill "$vite_pid" 2>/dev/null || true' EXIT
for _ in $(seq 1 60); do
  curl -sf "http://127.0.0.1:$port/" >/dev/null && break
  sleep 0.5
done

node demo-video/capture-ui.mjs "http://127.0.0.1:$port" "$work/ui"
node demo-video/capture-pill.mjs "http://127.0.0.1:$port" "$work/pill-frames"
node demo-video/music.mjs "$work/music.wav"
nice -n 10 node demo-video/render.mjs "$work" "$work/verenu-demo-60s.mp4"

echo "Done: $work/verenu-demo-60s.mp4"
