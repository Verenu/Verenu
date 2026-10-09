#!/usr/bin/env bash

# Explicit paths are relative to the caller; default jobs are atomically unique.
prepare_demo_work_dir() {
  local requested="${1:-}" cache_root="$2"
  if [[ -n "$requested" ]]; then
    mkdir -p -- "$requested"
    (cd -- "$requested" && pwd -P)
  else
    mkdir -p -- "$cache_root"
    mktemp -d "$cache_root/job-XXXXXX"
  fi
}
