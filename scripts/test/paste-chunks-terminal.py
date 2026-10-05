#!/usr/bin/env python3
"""Capture public synthetic bracketed pastes in a disposable terminal."""
import argparse
import json
import os
import select
import sys
import termios
import time
import tty
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--output", required=True, type=Path)
parser.add_argument("--timeout", type=float, default=180)
args = parser.parse_args()
saved = termios.tcgetattr(sys.stdin)
pending = b""
chunks = []
deadline = time.monotonic() + args.timeout
try:
    tty.setraw(sys.stdin)
    sys.stdout.write("\x1b[?2004hPublic synthetic paste fixture ready\r\n")
    sys.stdout.flush()
    while time.monotonic() < deadline:
        ready, _, _ = select.select([sys.stdin], [], [], min(1, max(0, deadline - time.monotonic())))
        if not ready:
            continue
        data = os.read(sys.stdin.fileno(), 65536)
        if not data or b"\x03" in data:
            break
        pending += data
        while b"\x1b[201~" in pending:
            block, pending = pending.split(b"\x1b[201~", 1)
            if b"\x1b[200~" not in block:
                raise ValueError("Input was not bracketed paste")
            block = block.split(b"\x1b[200~", 1)[1]
            chunks.append(block.decode("utf-8"))
            args.output.write_text(json.dumps({"chunks": chunks}), encoding="utf-8")
            sys.stdout.write(block.decode("utf-8").replace("\r", "\r\n"))
            sys.stdout.flush()
finally:
    sys.stdout.write("\x1b[?2004l")
    sys.stdout.flush()
    termios.tcsetattr(sys.stdin, termios.TCSADRAIN, saved)
print("\n[INFO] Synthetic paste fixture closed", flush=True)
