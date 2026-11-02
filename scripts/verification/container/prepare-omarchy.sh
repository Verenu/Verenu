#!/bin/sh
set -eu

if [ "$HOME" != /home/agent ] || [ -e /dev/snd ] || [ -z "${HYPRLAND_INSTANCE_SIGNATURE:-}" ]; then
  echo 'Run inside the isolated agent desktop.' >&2
  exit 2
fi
config="$HOME/.config/hypr/hyprland.lua"
bindings="$HOME/.config/hypr/bindings.lua"
[ -f "$config" ] || { echo 'Hyprland configuration is missing.' >&2; exit 1; }
[ -e "$bindings" ] || : > "$bindings"
if ! grep -Fqx "require('hypr.bindings')" "$config"; then
  printf "\nrequire('hypr.bindings')\n" >> "$config"
fi
hyprctl reload >/dev/null
errors=$(hyprctl configerrors)
case "$errors" in
  ''|ok) ;;
  *) printf '%s\n' "$errors" >&2; exit 1 ;;
esac
