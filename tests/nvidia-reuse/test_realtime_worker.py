import importlib.util
import io
import pathlib
import struct
import unittest

import numpy as np

WORKER = pathlib.Path(__file__).resolve().parents[2] / "src-tauri/resources/nvidia-reuse/realtime_worker.py"
spec = importlib.util.spec_from_file_location("realtime_worker", WORKER)
worker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(worker)


class IdentityModel:
    def reset(self):
        pass

    def frame(self, spectrum):
        return spectrum


class StreamingTests(unittest.TestCase):
    def test_reconstructs_irregular_and_short_recordings(self):
        rng = np.random.default_rng(2)
        for length in (0, 1, 319, 320, 321, 1279, 16001):
            with self.subTest(length=length):
                samples = rng.normal(0, 0.1, length).astype(np.float32)
                audio = worker.StreamingAudio(IdentityModel())
                chunks = [audio.push(samples[i:i + 137]) for i in range(0, length, 137)]
                chunks.append(audio.flush())
                np.testing.assert_allclose(np.concatenate(chunks), samples, atol=1e-6)
                with self.assertRaises(ValueError):
                    audio.push(samples)

    def test_reset_discards_previous_recording_state(self):
        audio = worker.StreamingAudio(IdentityModel())
        first = np.arange(900, dtype=np.float32) / 1000
        expected = np.concatenate((audio.push(first), audio.flush()))
        audio.reset()
        actual = np.concatenate((audio.push(first), audio.flush()))
        np.testing.assert_array_equal(actual, expected)

    def test_protocol_rejects_invalid_and_truncated_input(self):
        for data in (b"\x01", struct.pack("<I", worker.MAX_SAMPLES + 1), struct.pack("<I", 1),
                     struct.pack("<I", 1) + struct.pack("<f", float("nan"))):
            with self.subTest(data=data), self.assertRaises((ValueError, EOFError)):
                worker.serve(io.BytesIO(data), io.BytesIO(), worker.StreamingAudio(IdentityModel()))


if __name__ == "__main__":
    unittest.main()
