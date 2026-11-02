#!/bin/sh
set -eu
if [ "$HOME" != /home/agent ] || [ -e /dev/snd ] || [ -z "${HYPRLAND_INSTANCE_SIGNATURE:-}" ]; then
  echo 'Run inside the isolated agent desktop.' >&2
  exit 2
fi
set -- /tmp/tmp.*/pulse/native
if [ "$#" -ne 1 ] || [ ! -S "$1" ]; then
  echo 'Expected exactly one private audio socket.' >&2
  exit 1
fi
export PULSE_SERVER="unix:$1"
[ "$(pactl get-default-source)" = verenu_fixture.monitor ] || exit 1
[ "$(pactl list short sources | wc -l)" -eq 1 ] || exit 1
audio=$(mktemp --suffix=.wav)
playback=
keyboard=
cleanup() {
  [ -z "$playback" ] || kill "$playback" 2>/dev/null || true
  [ -z "$keyboard" ] || kill "$keyboard" 2>/dev/null || true
  [ -z "$keyboard" ] || wait "$keyboard" 2>/dev/null || true
  rm -f "$audio"
}
trap cleanup EXIT
trap 'exit 143' TERM HUP INT
espeak-ng -w "$audio" 'This is a controlled test of the microphone recording system'
log=$(find "$HOME/.config/Verenu/logs" -maxdepth 1 -name 'session-*.log' -printf '%T@ %p\n' | sort -rn | head -1 | cut -d ' ' -f 2-)
[ -n "$log" ] || { echo 'No Verenu session log found.' >&2; exit 1; }
if grep -Eq 'linux hotkey: portal press|lifecycle: recording -> stopping|pipeline: start gen=' "$log"; then
  echo 'Start a fresh Verenu session before running this fixture.' >&2
  exit 1
fi
offset=$(($(wc -c < "$log") + 1))
(sleep 1; paplay --device=verenu_fixture "$audio") & playback=$!
/tmp/verenu-hold-ctrl-super 5000 & keyboard=$!
i=0
while [ "$i" -lt 25 ]; do
  if tail -c "+$offset" "$log" | awk -f /project/scripts/verification/container/completed-generation.awk; then
    kill "$keyboard" 2>/dev/null || true
    wait "$keyboard"
    keyboard=
    break
  fi
  kill -0 "$keyboard" 2>/dev/null || { wait "$keyboard"; exit 1; }
  sleep 1
  i=$((i + 1))
done
[ -z "$keyboard" ] || { echo 'Dictation did not finish within 25 seconds; inspect the app and log.' >&2; exit 1; }
wait "$playback"
playback=
