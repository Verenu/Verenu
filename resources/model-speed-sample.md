The bundled `model-speed-sample.wav` contains synthetic English speech generated
with eSpeak NG at 155 words per minute, resampled to mono PCM16 at 16 kHz. It
contains no recording or user data.

Text: "Please send the meeting notes tomorrow morning. We will review the
project together and confirm the next steps."

The optional local model test runs this sample once, then three warm runs. It
reports the initial run separately and uses only warm timings for recommendations.
These timings measure speed, not recognition accuracy. Normal speech timings are
normalized to ten seconds of audio; cleanup timings are normalized to 100 input
characters. Metadata remains in memory for the current app process and is never
sent to analytics. Cold cleanup loads are included in ordinary cleanup timing.
