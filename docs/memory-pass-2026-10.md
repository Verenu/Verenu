# Memory reduction pass, October 2026

This pass audits the Rust audio and provider paths, retry state, local model
managers, SQLite history and Insights, diagnostics, frontend caches, and both
persistent WebViews. Changes stay on `t3code/optimize-memory-usage` in the assigned
worktree. No runtime dependency, model choice, audio quality setting, or model
unload policy changes.

Gemini and OpenRouter previously allocated a complete base64 string, then copied
it into the serialized JSON upload. `Base64Audio` now writes base64 directly
through Serde into a buffer reserved for the audio and escaped metadata. The
request drops its source object before waiting for the network. Multipart
providers already share WAV bytes and keep their existing upload path.

`audio-request-memory` measures peak live requested heap allocations with a
counting allocator. It compares the previous string-plus-JSON request with the
new production serialization helper and asserts byte-identical request bodies.
It makes no network requests and uses no credentials. The source WAV and client
initialization are excluded from both measurements.

| Synthetic PCM16 audio duration | Previous request allocations | New request allocations | Reduction |
| --- | ---: | ---: | ---: |
| 60 seconds | 7,680,982 bytes | 2,561,876 bytes | 66.6% |
| 15 minutes | 115,200,982 bytes | 38,401,876 bytes | 66.7% |

These are request allocations, not total app RAM or allocator overhead. The
numbers above use the OpenRouter-shaped fixture. Gemini uses the same helper,
with different metadata sizes. Request serialization is still buffered; it
does not stream audio over the network.

Completed captures now release unused vector capacity before sharing PCM with
the pipeline. A 60-second regression fixture with twice its required capacity
falls from 7,680,000 to 3,840,000 bytes without changing a sample. Actual savings
depend on capture growth and take length. Shrinking may move the allocation once
at handoff.

The pipeline also clears its shared WAV cache after transcription completes,
fails, or is cancelled. Cleanup and retained retries keep PCM. At 16 kHz mono,
this releases 1,920,044 cached bytes for a one-minute take, or 28,800,044 for a
15-minute take. Existing upload handles remain valid. A later cloud retry
encodes WAV again; regression coverage verifies shared owners and byte-identical
regeneration.

Browser mocks now live only in the lazily imported `tauri.dev.ts`. The existing
active mock implementation was copied exactly into that module. Once loaded,
browser calls still start synchronously so a settings save cannot race a view
reading it back. Native and authenticated live-session transports do not import
the mocks. The separate browser mock chunk is not loaded by native windows.

Secondary pages, settings sections, onboarding, optional dialogs, and the dev
panel now load when needed. Home and the settings controller remain eager. The
controller preserves native settings events, navigation guards, Escape, and
focus restoration. Search highlighting and heading focus wait for a section to
load. Regression coverage delays Audio settings by 1.1 seconds and verifies
search focus; a failed Developer import exercises the reload recovery action.
Reload is necessary because browsers can retain a failed module fetch for the
document's lifetime.

The development worker returns before importing `App.svelte`; previously it
loaded the complete UI graph despite never mounting it. Loaded component
factories are reused, but component instances still unmount normally. Visiting
every page eventually loads their code: this reduces initial loading rather
than unloading previously visited modules.

Controlled production previews use the same dark browser mock fixture with
setup completed and the legacy rail enabled. The mock chunk is included equally
in both measurements. Decoded JavaScript and CSS loaded by Home were:

| Resource | Before | After | Reduction |
| --- | ---: | ---: | ---: |
| JavaScript | 785,964 bytes | 337,446 bytes | 57.1% |
| CSS | 273,179 bytes | 79,842 bytes | 70.8% |

These measure code loading, not total app RAM. Native windows also avoid the
browser mock chunk, and a fresh setup loads onboarding when it is needed.

The Linux `webview-memory` example compares independent main/pill renderers
with [WebKit related views](https://webkitgtk.org/reference/webkit2gtk/stable/ctor.WebView.new_with_related_view.html).
It uses minimal HTML without application settings, credentials, or provider
calls. Both modes passed two window-specific IPC handshakes and three event
rounds, including events with the main window hidden and then shown. Content
managers remain distinct and the pill remains mapped. The related-view mode
used one renderer instead of two, but the saving was not stable: after the
hide/show cycle, one comparison measured 331,807 KiB median process-tree PSS
for independent views and 354,938 KiB for related views. A repeat of the latter
measured 325,913 KiB. Production renderer sharing was therefore rejected. The
example remains an opt-in measurement tool, not a production change or an
installed-app benchmark.

Other major allocations already have safeguards. Recording samples have a
15-minute limit and streaming resampling. History is paginated and frontend
history and icon caches are bounded. Diagnostics and recovery queues have
limits. Local STT and LLM managers already unload idle models and respond to
memory pressure. Linux already disables WebKit page/resource caching and trims
freed native allocations while idle. Model weights and the WebView runtimes
remain substantial memory costs. Reducing those further requires controlled
native measurements and explicit latency or model-quality tradeoffs.

Verification completed on Linux:

- `npm test`: all 32 fast-profile checks passed, including browser behavior,
  settings persistence, onboarding, state, accessibility, and performance.
- `cargo test --manifest-path src-tauri/Cargo.toml --lib`: 902 passed, 9 ignored.
- Frontend check and production build passed; all 236 frontend unit tests passed.
- The allocation example passed at one and fifteen minutes.
- The restarted isolated dev session passed all 9 checks with
  `VERENU_DEV_REQUIRE_LIVE=1`, including real synthetic speech through the
  production provider pipeline and persisted history.
- Visible browser inspection at 1280 × 800 and 390 × 844 covered Home,
  settings, and dev-test controls. Live-session resource inspection confirmed
  that browser mocks were not loaded. Screenshots were captured and inspected.

The follow-up fast suite used this worktree's own mock server on port 48719.
The default runner reused another session's server on port 1420, so that run
was discarded. A temporary verification preload redirected only Playwright's
initial localhost:1420 navigation to the owned server; assertions and frozen
smoke scripts were unchanged. The live dev-session backend was rebuilt and
restarted after Rust edits. The deferred-loading regression and UI timing
budgets passed against the owned mock server.

The screenshots use empty/public mock data. Phone captures use the app's Android
document marker in a production preview at 390 × 844; they are browser checks,
not native Android device verification. Each image passed privacy review before
upload.

| Surface | Before | After |
| --- | --- | --- |
| Desktop Home, 1280 × 800 | [Screenshot](https://files.luhtwin.xyz/temporary/file-28ead99ef6ff83e98108b744bd4a6eb8.png) | [Screenshot](https://files.luhtwin.xyz/temporary/file-4abf39fbe9e43ecd5febcfb076a454ac.png) |
| Phone General settings, 390 × 844 | [Screenshot](https://files.luhtwin.xyz/temporary/file-0d20ca5386a781662df8ad3a7ef231d2.png) | [Screenshot](https://files.luhtwin.xyz/temporary/file-05d01ff2e9818f45f902fdf8b6f7e864.png) |

`npm run lint` remains blocked by 11 existing Linux errors in unchanged files,
including unused items, title-bar argument counts, a needless return, and a
clipboard question-mark lint. Clippy for the application and measurement examples
passes with only those existing lint categories allowed on the command line.
No source allowances were added. An extra all-targets Clippy attempt also found
existing test/example warnings.

Windows and macOS native capture, permissions, injection, and installed-app RAM
comparisons were not performed. Live provider verification covers the configured
pipeline. Direct attempts through both Gemini transcription endpoints were
rejected with `API_KEY_INVALID` for the existing configured Google credential;
OpenRouter has no configured credential. The new JSON adapters have deterministic
serialization and header coverage. Original isolated-session settings were
restored after the attempts. Debug worker RAM varied across dictation and idle
maintenance, so no total idle-RAM percentage is claimed. Public uploads became
available through the configured native Linux uploader during the follow-up.

To reproduce the allocation measurements:

```bash
cargo run --manifest-path src-tauri/Cargo.toml --example audio-request-memory
cargo test --manifest-path src-tauri/Cargo.toml captured_audio --lib -- --nocapture
```

To reproduce the Linux renderer experiment on a working display:

```bash
WEBKIT_DMABUF_RENDERER_FORCE_SHM=1 cargo run --manifest-path src-tauri/Cargo.toml --example webview-memory -- separate
WEBKIT_DMABUF_RENDERER_FORCE_SHM=1 cargo run --manifest-path src-tauri/Cargo.toml --example webview-memory -- shared
```

Compare repeated runs under matching desktop conditions. The probe waits for
handshakes, exercises visibility, trims the native allocator, and reports the
median of three PSS samples. It exits with an error on missing handshakes,
events, or an unexpected renderer count.
