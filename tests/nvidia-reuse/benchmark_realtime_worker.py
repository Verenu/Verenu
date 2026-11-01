"""Pace synthetic audio through the research worker and compare one-shot latency."""

import argparse
import io
import json
import os
import select
import struct
import subprocess
import sys
import time

import numpy as np
import soundfile as sf


def read_exact(stream, size, deadline):
    data = bytearray()
    while len(data) < size:
        remaining = deadline - time.monotonic()
        if remaining <= 0 or not select.select([stream], [], [], remaining)[0]:
            raise TimeoutError("response timed out")
        chunk = os.read(stream.fileno(), size - len(data))
        if not chunk:
            raise EOFError("worker exited")
        data.extend(chunk)
    return data


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--offline-worker", required=True)
    parser.add_argument("--realtime-worker", required=True)
    parser.add_argument("--audio", help="16 kHz mono synthetic WAV")
    args = parser.parse_args()
    rate, seconds, block = 16000, 4, 6400
    if args.audio:
        samples, input_rate = sf.read(args.audio, dtype="float32")
        if input_rate != rate or samples.ndim != 1 or len(samples) == 0 or not np.isfinite(samples).all():
            raise ValueError("expected finite 16 kHz mono audio")
        samples = samples.astype("<f4")
    else:
        rng = np.random.default_rng(42)
        t = np.arange(rate * seconds, dtype=np.float32) / rate
        samples = (0.12 * np.sin(2 * np.pi * 220 * t) + 0.02 * rng.standard_normal(len(t))).astype("<f4")
    seconds = len(samples) / rate
    wav = io.BytesIO()
    sf.write(wav, samples, rate, format="WAV", subtype="PCM_16")
    before = time.monotonic()
    offline = subprocess.run([sys.executable, args.offline_worker], input=wav.getvalue(), stdout=subprocess.PIPE,
                             stderr=subprocess.PIPE, timeout=120, check=True)
    offline_time = time.monotonic() - before
    baseline = np.frombuffer(offline.stdout, dtype="<f4")
    start = time.monotonic()
    process = subprocess.Popen([sys.executable, args.realtime_worker], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                               stderr=subprocess.DEVNULL, bufsize=0)
    results = []

    def exchange(payload):
        view = memoryview(payload)
        while view:
            count = process.stdin.write(view)
            if not count:
                raise BrokenPipeError("worker input closed")
            view = view[count:]
        count = struct.unpack("<I", read_exact(process.stdout, 4, start + 180))[0]
        if count > len(samples):
            raise ValueError("invalid output length")
        results.append(np.frombuffer(read_exact(process.stdout, count * 4, start + 180), dtype="<f4"))

    try:
        for offset in range(0, len(samples), block):
            target = start + min(offset + block, len(samples)) / rate
            time.sleep(max(0, target - time.monotonic()))
            chunk = samples[offset:offset + block]
            exchange(struct.pack("<I", len(chunk)) + chunk.tobytes())
        exchange(struct.pack("<I", 0xFFFFFFFF))
        elapsed = time.monotonic() - start
        process.stdin.close()
        if process.wait(timeout=5):
            raise RuntimeError("worker failed")
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()
    output = np.concatenate(results)
    if len(output) != len(samples) or len(baseline) != len(samples) or not np.isfinite(output).all() or not np.isfinite(baseline).all():
        raise ValueError("invalid output")
    print(json.dumps({"audio_seconds": seconds, "send_block_seconds": block / rate,
                      "offline_after_recording_seconds": round(offline_time, 3),
                      "realtime_total_seconds": round(elapsed, 3),
                      "realtime_after_recording_seconds": round(max(0, elapsed - seconds), 3),
                      "samples_each": len(output), "finite": True,
                      "offline_rms": round(float(np.sqrt(np.mean(baseline ** 2))), 6),
                      "realtime_rms": round(float(np.sqrt(np.mean(output ** 2))), 6)}))


if __name__ == "__main__":
    main()
