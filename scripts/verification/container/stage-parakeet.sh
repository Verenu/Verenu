#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname "$0")/../../.." && pwd)
dir="$root/test-results/verification-assets"
archive="$dir/parakeet-v3-int8.tar.gz"
checksum=43d37191602727524a7d8c6da0eef11c4ba24320f5b4730f1a2497befc2efa77
mkdir -p "$dir"
verify() { printf '%s  %s\n' "$checksum" "$1" | sha256sum -c --status; }
if [ -f "$archive" ] && verify "$archive"; then
  printf '%s\n' "$archive"
  exit 0
fi
partial=$(mktemp "$dir/parakeet.XXXXXXXX")
trap 'rm -f "$partial"' EXIT
curl --fail --location --retry 2 --connect-timeout 15 \
  https://blob.handy.computer/parakeet-v3-int8.tar.gz -o "$partial"
verify "$partial" || { echo 'Model checksum mismatch.' >&2; exit 1; }
mv -f "$partial" "$archive"
printf '%s\n' "$archive"
