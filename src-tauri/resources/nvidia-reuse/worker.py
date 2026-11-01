import io
import json
import os
import sys

import numpy as np
import soundfile as sf
import torch
from safetensors.torch import load_file

root = os.path.join(os.path.dirname(__file__), "model")
sys.path.insert(0, root)
from models.generator_SEMamba_time_d4 import SEMamba
from models.stfts import mag_phase_stft, mag_phase_istft


def enhance(samples):
    with open(os.path.join(root, "config.json"), encoding="utf-8") as file:
        config = json.load(file)
    if not torch.cuda.is_available():
        raise RuntimeError("CUDA is unavailable")
    device = torch.device("cuda")
    model = SEMamba(cfg=config).to(device)
    model.load_state_dict(load_file(os.path.join(root, "model.safetensors"), device="cuda"))
    model.eval()
    stft = config["stft_cfg"]
    factor = config["model_cfg"]["compress_factor"]
    rate = stft["sampling_rate"]
    scale = lambda value: int(round(value * 16000 / rate / 2)) * 2
    n_fft, hop, win = (scale(stft[key]) for key in ("n_fft", "hop_size", "win_size"))
    chunk_size, overlap = 160000, 32000
    result = np.zeros(len(samples), dtype=np.float32)
    weights = np.zeros(len(samples), dtype=np.float32)
    for start in range(0, len(samples), chunk_size - overlap):
        chunk = samples[start:start + chunk_size]
        audio = torch.from_numpy(chunk).reshape(1, -1).to(device)
        with torch.inference_mode():
            mag, phase, _ = mag_phase_stft(audio, n_fft=n_fft, hop_size=hop, win_size=win, compress_factor=factor, center=True, addeps=False)
            amplitude, phase, _ = model(mag, phase)
            zero_portion = (torch.expm1(torch.relu(amplitude)) == 0).float().mean(dim=1)
            amplitude[:, :, (zero_portion > 0.5)[0]] = 0
            output = mag_phase_istft(amplitude, phase, n_fft, hop, win, factor).flatten()
            output = torch.nn.functional.pad(output[:len(chunk)], (0, max(0, len(chunk) - len(output))))
        output = output.cpu().numpy().astype(np.float32)
        window = np.ones(len(chunk), dtype=np.float32)
        fade = min(overlap, len(chunk))
        if start:
            window[:fade] = np.linspace(0, 1, fade, dtype=np.float32)
        if start + len(chunk) < len(samples):
            window[-fade:] = np.linspace(1, 0, fade, dtype=np.float32)
        result[start:start + len(chunk)] += output * window
        weights[start:start + len(chunk)] += window
        if start + len(chunk) == len(samples):
            break
    result /= np.maximum(weights, 1e-8)
    if len(result) != len(samples) or not np.isfinite(result).all():
        raise RuntimeError("Invalid model output")
    return result


def main():
    if sys.argv[1:] == ["--probe"]:
        samples = np.sin(np.arange(16000, dtype=np.float32) * 0.03) * 0.02
        enhanced = enhance(samples)
        if not np.any(enhanced):
            raise RuntimeError("Model returned silence during the self-test")
        print("ready")
        return
    audio, rate = sf.read(io.BytesIO(sys.stdin.buffer.read()), dtype="float32", always_2d=True)
    if rate != 16000 or audio.shape[1] != 1 or len(audio) == 0:
        raise RuntimeError("Expected 16 kHz mono audio")
    sys.stdout.buffer.write(enhance(audio[:, 0]).tobytes())


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(f"RE-USE worker failed: {type(error).__name__}: {error}", file=sys.stderr)
        sys.exit(1)
