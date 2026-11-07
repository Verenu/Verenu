#!/bin/sh
set -eu
if [ "$HOME" != /home/agent ] || [ -e /dev/snd ] || [ -z "${HYPRLAND_INSTANCE_SIGNATURE:-}" ]; then
  echo 'Run inside the isolated agent desktop.' >&2
  exit 2
fi
source=/project/test-results/verification-assets/parakeet-tdt-0.6b-v3-int8
verify() {
  printf '%s  %s\n' "$1" "$source/$2" | sha256sum -c --status || {
    echo 'Stage the verified model on the host first.' >&2; exit 1;
  }
}
verify 666903c76b9798caf2c210afd4f6cd60b08a8dbf9800ec8d7a3bc0d2148ac466 config.json
verify eea7483ee3d1a30375daedc8ed83e3960c91b098812127a0d99d1c8977667a70 decoder_joint-model.int8.onnx
verify 6139d2fa7e1b086097b277c7149725edbab89cc7c7ae64b23c741be4055aff09 encoder-model.int8.onnx
verify a9fde1486ebfcc08f328d75ad4610c67835fea58c73ba57e3209a6f6cf019e9f nemo128.onnx
verify d58544679ea4bc6ac563d1f545eb7d474bd6cfa467f0a6e2c1dc1c7d37e3c35d vocab.txt
root="$HOME/.config/Verenu/models/stt"
target="$root/parakeet-tdt-0.6b-v3-int8"
mkdir -p "$root"
if [ -e "$target" ]; then
  echo 'Model already exists in this desktop; refusing to replace it.' >&2
  exit 1
fi
staging=$(mktemp -d "$root/.parakeet.XXXXXXXX")
trap 'rm -rf "$staging"' EXIT
mkdir "$staging/parakeet-tdt-0.6b-v3-int8"
cp "$source"/* "$staging/parakeet-tdt-0.6b-v3-int8/"
mv "$staging/parakeet-tdt-0.6b-v3-int8" "$target"
echo 'Installed verified Parakeet V3 in the isolated profile.'
