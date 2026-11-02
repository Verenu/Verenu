#!/bin/sh
set -eu
if [ "$HOME" != /home/agent ] || [ -e /dev/snd ] || [ -z "${HYPRLAND_INSTANCE_SIGNATURE:-}" ]; then
  echo 'Run inside the isolated agent desktop.' >&2
  exit 2
fi
archive=/project/test-results/verification-assets/parakeet-v3-int8.tar.gz
checksum=43d37191602727524a7d8c6da0eef11c4ba24320f5b4730f1a2497befc2efa77
printf '%s  %s\n' "$checksum" "$archive" | sha256sum -c --status || {
  echo 'Stage the verified model on the host first.' >&2; exit 1;
}
root="$HOME/.config/Verenu/models/stt"
target="$root/parakeet-tdt-0.6b-v3-int8"
mkdir -p "$root"
if [ -e "$target" ]; then
  echo 'Model already exists in this desktop; refusing to replace it.' >&2
  exit 1
fi
staging=$(mktemp -d "$root/.parakeet.XXXXXXXX")
trap 'rm -rf "$staging"' EXIT
tar --no-same-owner --no-same-permissions --warning=no-unknown-keyword -xzf "$archive" -C "$staging"
[ -f "$staging/parakeet-tdt-0.6b-v3-int8/encoder-model.int8.onnx" ] || {
  echo 'Model archive has an unexpected layout.' >&2; exit 1;
}
mv "$staging/parakeet-tdt-0.6b-v3-int8" "$target"
echo 'Installed verified Parakeet V3 in the isolated profile.'
