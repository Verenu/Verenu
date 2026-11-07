#!/bin/sh
# Stages the Parakeet V3 model files from Hugging Face at a pinned commit.
set -eu
root=$(CDPATH= cd -- "$(dirname "$0")/../../.." && pwd)
dir="$root/test-results/verification-assets/parakeet-tdt-0.6b-v3-int8"
base='https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/8f23f0c03c8761650bdb5b40aaf3e40d2c15f1ce'
mkdir -p "$dir"
fetch() {
  name=$1
  checksum=$2
  target="$dir/$name"
  if [ -f "$target" ] && printf '%s  %s\n' "$checksum" "$target" | sha256sum -c --status; then
    return 0
  fi
  partial=$(mktemp "$dir/$name.XXXXXXXX")
  trap 'rm -f "$partial"' EXIT
  curl --fail --location --retry 2 --connect-timeout 15 "$base/$name" -o "$partial"
  printf '%s  %s\n' "$checksum" "$partial" | sha256sum -c --status || { echo "Checksum mismatch: $name" >&2; exit 1; }
  mv -f "$partial" "$target"
  trap - EXIT
}
fetch config.json 666903c76b9798caf2c210afd4f6cd60b08a8dbf9800ec8d7a3bc0d2148ac466
fetch decoder_joint-model.int8.onnx eea7483ee3d1a30375daedc8ed83e3960c91b098812127a0d99d1c8977667a70
fetch encoder-model.int8.onnx 6139d2fa7e1b086097b277c7149725edbab89cc7c7ae64b23c741be4055aff09
fetch nemo128.onnx a9fde1486ebfcc08f328d75ad4610c67835fea58c73ba57e3209a6f6cf019e9f
fetch vocab.txt d58544679ea4bc6ac563d1f545eb7d474bd6cfa467f0a6e2c1dc1c7d37e3c35d
printf '%s\n' "$dir"
