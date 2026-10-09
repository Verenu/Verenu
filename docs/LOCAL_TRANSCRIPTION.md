# Local transcription and cleanup

Verenu supports on-device transcription and on-device cleanup. Downloaded models and the local cleanup runtime are installed from Settings -> Models when you choose them. Apple Speech instead uses macOS speech permissions and system language assets.

## How the local path works

1. Verenu records the audio on your device.
2. A selected `local/<model>` transcription model receives the captured audio through the local STT adapter.
3. The local model returns raw transcript text.
4. With Cleanup Off, Verenu keeps that text as-is. With cleanup enabled, it sends the text to either a downloaded local cleanup model or a cloud cleanup provider.
5. Verenu pastes the final text and stores the dictation in local history.

Local transcription with local cleanup keeps the dictation data on the device after the model files have been downloaded. Local transcription with cloud cleanup keeps the audio local but sends the transcript and cleanup context to the selected cloud provider.

## Local transcription models

The built-in local transcription catalog currently includes:

- Parakeet V3 and Parakeet V2
- Moonshine Tiny, Base, Small, and Medium
- SenseVoice
- GigaAM V3
- Canary 180M Flash and Canary 1B V2
- Cohere
- Whisper Small and Large v3 Turbo, using portable CPU whisper.cpp on desktop
- Apple Speech on macOS, requiring on-device recognition for the selected locale
- FluidAudio Parakeet Ultra, 110M English, and Japanese on macOS 14 or later on Apple Silicon

The model picker shows download state, verifies completed downloads before marking them ready, and allows a downloaded model to be cancelled or removed. Apple Speech has no Verenu model download or delete action. It requests speech recognition permission when used and reports unavailable on-device locale assets instead of using Apple's cloud recognition.

Whisper downloads verify pinned SHA256 hashes. FluidAudio downloads verify every file against a pinned revision, size, and SHA256 hash. Its optional English CTC vocabulary booster is a separate download, not a transcription model. Acoustic rescoring runs only for explicitly English dictation with canonical terms from the captured Context; automatic language and Japanese skip it. Primary recognition can run without the booster.

Speech adapters receive one bounded immutable snapshot of the captured Context's canonical vocabulary. Correction strings and other Contexts' terms are excluded. Portable Whisper does not use WhisperKit or the Apple Neural Engine.

## Local cleanup models

The built-in local cleanup catalog currently includes:

- Gemma 4 E2B and E4B
- Qwen 2.5 0.5B, 1.5B, 3B, and 7B Instruct
- Phi-3 Mini 4K Instruct
- SmolLM2 360M and 1.7B Instruct
- Granite 3.3 2B and 8B Instruct

All local cleanup models share one downloaded runtime. Verenu downloads that runtime once, then manages the individual cleanup model files separately.

## Storage and downloads

- Local transcription models are stored in the app-data `models/stt` directory.
- Local cleanup models are stored in the app-data `models/cleanup` directory.
- The local cleanup runtime is stored in the app-data `models/bin` directory.
- Downloads are verified and installed atomically before a model becomes selectable.
- Settings -> Models provides download, cancel, and delete actions for local models and the cleanup runtime.

## Fallback behavior

- A missing selected model produces a download prompt in the model picker.
- A retryable local transcription failure can move to a configured cloud transcription fallback.
- A non-retryable local model or configuration error is reported instead of silently switching to the cloud path.
- Cloud fallback models still require the corresponding provider key.

## Platform limits

Local model downloads and inference are available on Windows, Linux, and Apple
Silicon Macs. On Intel Macs, the existing ONNX transcription and local cleanup
gate remains in place. Apple Speech and CPU Whisper are offered separately;
this does not validate the gated engines. FluidAudio requires macOS 14 or later
on Apple Silicon. These new speech adapters are unavailable on Android. The
frontend combines the general `local_models_supported_on_this_platform` gate
with each speech adapter's platform constraints; availability does not guarantee
that every model fits the device's memory.

Verification remains incomplete for Intel Mac and Windows Whisper, Apple Speech
permissions and recognition, FluidAudio Ultra/Japanese/CTC rescoring, and the
packaged Mac application path. Linux CPU Whisper and the Mac 110M helper have
actual inference evidence; a helper run does not establish packaged app behavior.

Speed, memory use, and output quality depend on the selected model and the computer running it. Larger local cleanup models need more memory and may take longer to answer.

## Choosing a private path

For a fully on-device dictation path, select a local transcription model, select a local cleanup model or turn Cleanup Off, and download the required files. Model downloads and any normal update or service-status requests are separate from dictation processing.

See [Privacy & Data](PRIVACY_SUMMARY.md) for the short privacy summary and [Data And Privacy](DATA_AND_PRIVACY.md) for the complete data map.
