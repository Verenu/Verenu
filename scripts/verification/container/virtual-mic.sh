#!/bin/sh
set -eu

if [ "$#" -eq 0 ]; then
  echo 'Usage: virtual-mic.sh COMMAND [ARG ...]' >&2
  exit 2
fi
if [ ! -f /.dockerenv ] || [ -e /dev/snd ]; then
  echo 'Run inside a disposable Docker container without host audio devices.' >&2
  exit 2
fi

runtime=$(mktemp -d)
chmod 700 "$runtime"
export HOME="$runtime" XDG_RUNTIME_DIR="$runtime" PULSE_RUNTIME_PATH="$runtime/pulse"
export PULSE_SERVER="unix:$runtime/native"
mkdir -m 700 "$PULSE_RUNTIME_PATH"
cleanup() {
  pulseaudio --kill >/dev/null 2>&1 || true
  rm -rf "$runtime"
}
trap cleanup EXIT HUP INT TERM

pulseaudio --daemonize=yes --exit-idle-time=-1 --disallow-exit=no -n \
  --load="module-native-protocol-unix socket=$runtime/native auth-anonymous=1" \
  --load="module-null-sink sink_name=verenu_fixture"
pactl set-default-source verenu_fixture.monitor
"$@"
