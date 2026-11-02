#!/bin/sh
set -eu

cd "$(dirname "$0")"
exec sh ./virtual-mic.sh sh -c '
  set -eu
  [ "$(pactl get-default-source)" = verenu_fixture.monitor ]
  [ "$(pactl list short sources | wc -l)" -eq 1 ]
  espeak-ng -w "$HOME/speech.wav" "This is a synthetic microphone test"
  timeout 7 parec --device=verenu_fixture.monitor --file-format=wav "$HOME/capture.wav" &
  capture=$!
  sleep 1
  paplay --device=verenu_fixture "$HOME/speech.wav"
  wait "$capture" || [ "$?" -eq 124 ]
  ffmpeg -v error -i "$HOME/capture.wav" -f s16le -ac 1 -ar 16000 "$HOME/capture.raw"
  python3 -c '\''import array, pathlib, sys
samples = array.array("h", pathlib.Path(sys.argv[1]).read_bytes())
assert samples and max(map(abs, samples)) > 1000, "captured audio is silent"
print("Private synthetic microphone loopback passed")'\'' "$HOME/capture.raw"
'
