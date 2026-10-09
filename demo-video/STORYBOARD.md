# Verenu 60-second demo: storyboard

1920×1080, 30 fps, 60.0 s. Music only, no narration. The original music bed
runs at 100 BPM, and each bar is 2.4 s. Every cut lands on a bar line.
Timings below match `S` in `stage.html`.

| Time | Scene | On screen | Source |
| --- | --- | --- | --- |
| 0.0–4.8 | **Title** | The five-bar mark grows in. Then "Verenu", *Hold a hotkey. Talk. Release.*, and *Free, open-source AI dictation for Windows, macOS, and Linux*. | Brand mark from `src-tauri/icons/verenu-mark.svg`. Fraunces, Inter Tight, and JetBrains Mono come from the app's own font packages. |
| 4.8–16.8 | **Dictate into any app** | An illustrated chat app. The keycaps `Ctrl` `Super` press, and the **real dictation pill** shows recording bars and the "Team chat" context chip. The spoken words appear, filler words included. On release the pill rolls through Transcribing… → Cleaning… → Pasting…, and the cleaned sentence lands in the message box all at once, matching clipboard paste. Then it sends. The headline changes to *Clean text, right where you were typing*. | The pill comes from `src/PillApp.svelte`, driven by the backend's event names (`capture-pill.mjs`). The chat app is an illustration and is labeled as one. |
| 16.8–24.0 | **Style** | Real Style page. A slow push-in highlights the four cleanup levels (Off, Light, Medium, Strong), then the tone cards. | `Style.svelte` |
| 24.0–33.6 | **Contexts** | The Team chat context, with its Apps & sites chips and the sidebar context list highlighted. A cut to the Development context highlights its vocabulary list (SQLite, Kubernetes, Tauri, Verenu). | `Contexts.svelte` with seeded sample contexts |
| 33.6–40.8 | **Models** | The real Models page. Cloud presets are highlighted, then the Local AI presets ("Runs entirely on your device, private and offline"). | Settings › Models |
| 40.8–48.0 | **Local history and insights** | Home history (sample dictations), then Insights (pace, words, changes). | `Home.svelte` and `Insights.svelte` with sample data |
| 48.0–55.2 | **Yours, end to end** | Four cards: Free and open source (MIT, no subscriptions). Local-first (keys in the OS credential store). Your providers, or none (Groq/OpenAI/Google BYOK, or fully offline). Windows · macOS · Linux (Tauri, not Electron). | Claims from `README.md` |
| 55.2–60.0 | **End card** | Mark, "Verenu", *Free, open-source AI dictation*, `github.com/Verenu/Verenu`, and a disclosure line. It fades to black. | |

## Music

`music.mjs` synthesizes the whole track. It uses no samples or third-party
audio. The chords are Cmaj9 – Am7 – Fmaj7 – G6sus:

- Bars 1–2: a soft pad intro under the title.
- 4.8 s: kick, snap, hats, bass, and a sixteenth-note arpeggio enter with the first product shot.
- 55.2 s: the drums drop out, and a held final chord rings under the end card.

The render normalizes the mix to −16 LUFS integrated, with a −1.5 dBTP ceiling.

## Honesty notes

- App screens come from the real Svelte UI of this checkout, running in
  browser preview mode (`src/lib/tauri.dev.ts`). They are not mock-ups. The
  data is synthetic: contexts, vocabulary, snippets, and history are seeded in
  `capture-ui.mjs`. No personal data is used.
- The pill is the real component, fed a synthetic audio envelope through its
  real event interface. It is not a recording of a live dictation.
- The chat app in scene 2 is an illustration and is labeled as one on screen.
- On-screen claims are limited to what `README.md` and the app UI state.
