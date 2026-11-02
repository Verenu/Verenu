#!/bin/sh
set -eu
if [ "$HOME" != /home/agent ] || [ -e /dev/snd ] || [ -z "${HYPRLAND_INSTANCE_SIGNATURE:-}" ]; then
  echo 'Run inside the isolated agent desktop.' >&2
  exit 2
fi
source_dir=$(CDPATH= cd -- "$(dirname "$0")" && pwd)
build=$(mktemp -d)
trap 'rm -rf "$build"' EXIT
wayland-scanner client-header "$source_dir/virtual-keyboard-unstable-v1.xml" "$build/virtual-keyboard-client.h"
wayland-scanner private-code "$source_dir/virtual-keyboard-unstable-v1.xml" "$build/protocol.c"
cc -std=c11 -Wall -Wextra -Werror -I "$build" "$source_dir/hold-ctrl-super.c" "$build/protocol.c" \
  -o /tmp/verenu-hold-ctrl-super $(pkg-config --cflags --libs wayland-client xkbcommon)
echo /tmp/verenu-hold-ctrl-super
