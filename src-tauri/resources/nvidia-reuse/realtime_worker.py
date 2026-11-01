"""Research-only streaming worker for NVIDIA Real-time RE-USE.

The caller supplies a separately downloaded, pinned NVIDIA model directory via
VERENU_REALTIME_MODEL_DIR. Framed input and output are little-endian u32 counts
followed by 16 kHz mono float32 samples. A zero count resets state; 0xffffffff
flushes the recording. Responses use the same framing. EOF exits.
"""

import json
import os
import struct
import sys

import numpy as np

RATE = 16000
HOP = 320
WINDOW = 640
MAX_SAMPLES = 160000
FLUSH = 0xFFFFFFFF


def read_exact(stream, size):
    data = bytearray()
    while len(data) < size:
        part = stream.read(size - len(data))
        if not part:
            raise EOFError("truncated frame")
        data.extend(part)
    return data


class StreamingAudio:
    def __init__(self, model):
        self.model = model
        self.window = np.hanning(WINDOW + 1)[:-1].astype(np.float32)
        self.reset()

    def reset(self):
        self.model.reset()
        self.pending = np.zeros(HOP, dtype=np.float32)
        self.tail = np.zeros(HOP, dtype=np.float32)
        self.tail_weight = np.zeros(HOP, dtype=np.float32)
        self.received = 0
        self.emitted = 0
        self.skip = HOP
        self.finished = False

    def push(self, samples):
        if self.finished:
            raise ValueError("reset required after flush")
        samples = np.asarray(samples, dtype=np.float32)
        if samples.ndim != 1 or not np.isfinite(samples).all():
            raise ValueError("invalid samples")
        self.received += len(samples)
        return self._process(samples)

    def _process(self, samples):
        self.pending = np.concatenate((self.pending, samples))
        output = []
        while len(self.pending) >= WINDOW:
            spectrum = np.fft.rfft(self.pending[:WINDOW] * self.window)
            enhanced = self.model.frame(spectrum)
            if enhanced.shape != spectrum.shape or not np.isfinite(enhanced).all():
                raise ValueError("invalid model output")
            waveform = np.fft.irfft(enhanced, n=WINDOW).astype(np.float32) * self.window
            weight = self.window ** 2
            block = (waveform[:HOP] + self.tail) / np.maximum(weight[:HOP] + self.tail_weight, 1e-8)
            self.tail = waveform[HOP:]
            self.tail_weight = weight[HOP:]
            self.pending = self.pending[HOP:]
            if self.skip:
                self.skip -= HOP
            else:
                output.append(block)
        result = np.concatenate(output) if output else np.empty(0, dtype=np.float32)
        remaining = max(0, self.received - self.emitted)
        result = result[:remaining]
        self.emitted += len(result)
        return result

    def flush(self):
        if self.finished:
            raise ValueError("recording already flushed")
        result = self._process(np.zeros(WINDOW + HOP, dtype=np.float32))
        self.finished = True
        if self.emitted != self.received:
            raise ValueError("incomplete output")
        return result


class NvidiaModel:
    def __init__(self, path):
        import torch
        import huggingface_hub
        from safetensors.torch import load_file

        self.torch = torch
        if not torch.cuda.is_available():
            raise RuntimeError("CUDA is unavailable")
        sys.path.insert(0, path)
        # NVIDIA's reference module resolves its config during import. Keep it local.
        original = huggingface_hub.hf_hub_download
        huggingface_hub.hf_hub_download = lambda **kwargs: os.path.join(path, kwargs["filename"])
        try:
            import online_inference as reference
        finally:
            huggingface_hub.hf_hub_download = original
        with open(os.path.join(path, "config.json"), encoding="utf-8") as file:
            config = json.load(file)
        model = reference.SEMamba_decoder_list(cfg=config).cuda().eval()
        model.load_state_dict(load_file(os.path.join(path, "model.safetensors"), device="cuda"))
        self.model = model
        self.streaming = reference.StreamingSEMamba(model).eval()

    def reset(self):
        self.streaming.reset(161)

    def frame(self, spectrum):
        torch = self.torch
        with torch.inference_mode():
            spec = torch.as_tensor(spectrum, dtype=torch.complex64, device="cuda")
            magnitude = torch.log1p(torch.abs(spec)).reshape(1, -1, 1)
            phase = torch.angle(spec).reshape(1, -1, 1)
            mag, pha = self.streaming(magnitude, phase, 8, 0)
            mag = torch.expm1(torch.relu(mag))
            if (mag == 0).float().mean().item() > 0.5:
                mag = torch.zeros_like(mag)
            return torch.polar(mag, pha).flatten().cpu().numpy()


def serve(source, sink, audio):
    while True:
        header = source.read(4)
        if not header:
            return
        if len(header) < 4:
            header += read_exact(source, 4 - len(header))
        count = struct.unpack("<I", header)[0]
        if count == 0:
            audio.reset()
            result = np.empty(0, dtype=np.float32)
        elif count == FLUSH:
            result = audio.flush()
        elif count <= MAX_SAMPLES:
            samples = np.frombuffer(read_exact(source, count * 4), dtype="<f4")
            result = audio.push(samples)
        else:
            raise ValueError("invalid frame length")
        result = np.asarray(result, dtype="<f4")
        sink.write(struct.pack("<I", len(result)))
        sink.write(result.tobytes())
        sink.flush()


if __name__ == "__main__":
    try:
        model_path = os.path.realpath(os.environ["VERENU_REALTIME_MODEL_DIR"])
        audio = StreamingAudio(NvidiaModel(model_path))
        if sys.argv[1:] == ["--probe"]:
            samples = np.sin(np.arange(RATE, dtype=np.float32) * 0.03) * 0.02
            output = np.concatenate((audio.push(samples), audio.flush()))
            if len(output) != len(samples) or not np.isfinite(output).all():
                raise ValueError("invalid self-test output")
            print("ready")
        elif not sys.argv[1:]:
            serve(sys.stdin.buffer, sys.stdout.buffer, audio)
        else:
            raise ValueError("unknown argument")
    except Exception as error:
        print(f"Real-time RE-USE worker failed: {type(error).__name__}: {error}", file=sys.stderr)
        sys.exit(1)
