#!/bin/sh
set -eu

if [ "$#" -eq 0 ] || [ -e /dev/snd ]; then
  echo 'Run COMMAND in an isolated desktop without host audio devices.' >&2
  exit 2
fi

if [ -n "${WAYLAND_DISPLAY:-}" ] && [ "${WAYLAND_DISPLAY#/}" = "$WAYLAND_DISPLAY" ]; then
  WAYLAND_DISPLAY="$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY"
  export WAYLAND_DISPLAY
fi
runtime=$(mktemp -d)
chmod 700 "$runtime"
export PIPEWIRE_RUNTIME_DIR="$runtime"
export PULSE_RUNTIME_PATH="$runtime/pulse" PULSE_SERVER="unix:$runtime/pulse/native"
pw= wp= pp=
cleanup() {
  for pid in "$pp" "$wp" "$pw"; do
    [ -z "$pid" ] || kill "$pid" 2>/dev/null || true
  done
  for pid in "$pp" "$wp" "$pw"; do
    [ -z "$pid" ] || wait "$pid" 2>/dev/null || true
  done
  rm -rf "$runtime"
}
trap cleanup EXIT
trap 'exit 143' TERM HUP INT
pipewire >"$runtime/pipewire.log" 2>&1 & pw=$!
wireplumber >"$runtime/wireplumber.log" 2>&1 & wp=$!
pipewire-pulse >"$runtime/pulse.log" 2>&1 & pp=$!
i=0
until pactl info >/dev/null 2>&1; do
  i=$((i + 1))
  if [ "$i" -ge 30 ]; then
    cat "$runtime"/*.log >&2
    exit 1
  fi
  sleep .2
done
pactl load-module module-null-sink sink_name=verenu_fixture >/dev/null
pactl set-default-source verenu_fixture.monitor
if [ "$(pactl list short sources | wc -l)" -ne 1 ] ||
   [ "$(pactl get-default-source)" != verenu_fixture.monitor ]; then
  echo 'Unexpected audio sources in private server.' >&2
  exit 1
fi
"$@"
