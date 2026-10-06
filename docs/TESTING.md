# Testing

Use the smallest test pass that covers the change, then run broader checks before release or risky PRs.

## Linux Context detection

Run the browser matrix inside a disposable Hyprland/Wayland compositor with
Chrome and a working AT-SPI registry. Give its D-Bus session its own short
`XDG_RUNTIME_DIR` and clear an inherited outer `DISPLAY` so accessibility
services do not share the desktop's sockets. Keep any nested compositor window
on a silent workspace without initial focus.

```bash
VERENU_CONTEXT_TEST_ISOLATED=1 node scripts/test-linux-contexts.mjs
VERENU_CONTEXT_FIXTURE_PID=PID cargo test --manifest-path src-tauri/Cargo.toml live_linux_window_matches_executable_context --lib -- --ignored
```

The browser suite creates its own profile and local public fixture websites.
It checks cold accessibility activation, tab changes, and two browser windows
sharing one PID. URL-shaped page inputs cannot supply website matches. It
focuses only fixture windows inside that compositor and closes its own browser.
It refuses to launch without the isolation opt-in. The app-alias check is
read-only and can run against a live desktop process. Set
`VERENU_CHROME_BINARY` for a different Chrome installation. Reports stay in
ignored `test-results/linux-contexts-*`. The app-alias check uses an isolated
database and requires a live window whose class differs from its executable,
such as T3 Code.

Context resolution retains the window class and title captured at recording
start. Linux website detection reads only browser chrome and verifies the
Hyprland address before and after the read. Ambiguous accessibility frames,
focus changes, unsupported browser pages, and unavailable AT-SPI fall back to
the captured app Context. These checks do not prove native shortcuts, audio
capture, or clipboard insertion, and Chrome results do not establish Firefox
or other browser behavior.

## Linux webview memory policy verification

Run with a working Linux display:

```bash
cargo run --manifest-path src-tauri/Cargo.toml --example linux-memory-policy
```

This checks the actual WebKit cache settings without opening the app database,
registering hotkeys, or calling providers. It runs on the process main thread
because WebKit shutdown can abort after initialization on a Rust test worker.

For RAM comparisons, use the same build profile, settings, window sizes, and
model state. Sum `Pss` in `/proc/<pid>/smaps_rollup` for the app and descendants,
as the app counter does. Compare startup, repeated navigation, and idle memory
after dictation or model unload. The native allocator trim runs at most once
every 30 seconds while dictation is idle and does not unload active models.

For request-allocation comparisons and capture-buffer regression coverage:

```bash
cargo run --manifest-path src-tauri/Cargo.toml --example audio-request-memory
cargo test --manifest-path src-tauri/Cargo.toml captured_audio --lib
```

The allocation example compares buffered JSON serialization with synthetic
audio and no provider calls. It measures request allocations, not total app RAM.

An opt-in WebKit renderer experiment compares independent and related views:

```bash
WEBKIT_DMABUF_RENDERER_FORCE_SHM=1 cargo run --manifest-path src-tauri/Cargo.toml --example webview-memory -- separate
WEBKIT_DMABUF_RENDERER_FORCE_SHM=1 cargo run --manifest-path src-tauri/Cargo.toml --example webview-memory -- shared
```

Use a working Linux display and repeat runs under matching conditions. The probe
checks IPC handshakes and visibility events, then reports process-tree PSS.
It uses minimal HTML; it does not measure the installed app or change production
renderer behavior.

## Linux cursor formatting verification

Run a disposable GTK entry in one terminal (requires Python PyGObject and GTK3):

```bash
env -u NO_AT_BRIDGE python scripts/test/linux-format-fixture.py
```

Keep its window focused and run this from another terminal, replacing `PID` with the fixture's printed process ID:

```bash
VERENU_FORMAT_FIXTURE_PID=PID cargo test --manifest-path src-tauri/Cargo.toml atspi_live_formats_disposable_entry --lib -- --ignored
```

This verifies real AT-SPI Collection discovery and cursor formatting for empty fields, continuation text, sentence endings, and existing whitespace. It changes only the disposable entry; it does not paste or call providers. Close the window afterward; it also closes after five minutes.

## Auto-learn verification

The shared regression matrix uses the detector's real confidence scores and
checks how many independent dictations each correction needs before promotion:

```bash
cargo test --manifest-path src-tauri/Cargo.toml api::auto_learn --lib
```

On a live Hyprland desktop with Python GTK3 installed, also run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml auto_learn_native_disposable_fields --lib -- --ignored
```

This test launches and closes its own disposable GTK window, removes
`NO_AT_BRIDGE` only from that process, and exercises native focused-text reads,
same-control identity, stable edits, real promotion, vocabulary prompt reuse,
and exclusion of password fields. It uses synthetic text and an in-memory
database. It does not capture audio, paste into other applications, or call
providers. The browser dev-session bridge deliberately disables automatic
learning, so browser tests do not replace this native check.

Windows UI Automation and macOS Accessibility still need native editor checks
on those systems before claiming platform verification. The PR Rust matrix
builds and tests the platform code on Windows and macOS.

## Linux hotkey gesture verification

For portal identity regressions, build the disposable native probe and run it
outside the editor's inherited app scope:

```bash
cargo build --manifest-path src-tauri/Cargo.toml --example linux-shortcut-portal
systemd-run --user --collect --wait --pipe --unit=verenu-portal-verification \
  "$PWD/src-tauri/target/debug/examples/linux-shortcut-portal" --keyboard
```

The probe uses the production portal connection code. Both connection rounds
must register Verenu's own identity, bind a disposable portal action, and receive
activation through Hyprland. `--keyboard` temporarily binds an unused
Ctrl+Alt+Shift+Super+F12 and drives it with `wtype`, then disables only its own
binding. It fails if the chord is occupied. It never records audio or reads the
clipboard. Without `--keyboard`, it checks compositor dispatch only.

An editor-launched probe alone cannot catch missing identity registration:
its process can inherit the editor's app identity. The browser error/recovery
test exercises the shortcut status event contract; browser sessions do not
register native shortcuts.

For Linux hotkey gesture changes, run the Rust classifier tests and the generated
Hyprland Lua fixture. The fixture requires `lua` and uses a fake compositor;
it does not change desktop bindings, capture audio, or call providers.

```bash
cargo test --manifest-path src-tauri/Cargo.toml hotkey --lib
cargo test --manifest-path src-tauri/Cargo.toml core::hyprland::tests --lib -- --include-ignored
```

## Default Gate

For task acceptance, source-bound evidence, real sessions, native scope, and
agent evaluations, read [AGENT_VERIFICATION.md](AGENT_VERIFICATION.md).

```bash
npm test
```

This runs the OnePyFone fast profile. It is deterministic and CI-friendly: no live APIs, no microphone capture, no OS permission prompts, and no real clipboard injection. Isolated browser tests run in parallel by default. Every run writes `test-results/onepyfone.json` unless `--no-json-report` is passed.

## Common Local Checks

```bash
npm run check
npm run lint
npm run build
npm run test:rust
npm run test:smoke
```

`npm run lint` runs frontend type-checking plus Rust Clippy with warnings denied.

## OnePyFone Profiles

```bash
npm run test:all
npm run test:full
npm run test:live
npm run test:native
npm run test:quality
npm run test:prompt
```

| Profile | Purpose |
| --- | --- |
| `fast` | Default deterministic suite for unit, compile, backend, UI, accessibility, state, and performance regressions |
| `live` | Configured-provider transcription and semantic prompt checks; skips when credentials or the optional WAV fixture are absent |
| `native` | Actual isolated native WebView and IPC checks |
| `native-prerequisites` | Configuration presence only; no behavior verification |
| `full` | Fast, live, and native profiles |

You can target suites directly:

```bash
python3 tests/OnePyFone.py --suite ui,state
python3 tests/OnePyFone.py --test accessibility.settings-focus
python3 tests/OnePyFone.py --suite ui,animation --workers 3 --fresh-server
python3 tests/OnePyFone.py --list
```

Use `--sequential` when investigating a timing or interaction failure. `--test` matches a stable test ID, display name, or category. Each registered process has a timeout, and the runner terminates its child process tree if that timeout expires.

## Results and performance baselines

The terminal summary names the failed contract, expected behavior, observed behavior, likely regression area, and whether the failure came from product behavior or test infrastructure. The JSON report uses schema version 2 and records the same fields for agents, plus status, measurements, checked baselines, duration, attempts, required or optional state, and Git metadata. Pass `--junit-report <path>` when a CI system also needs JUnit XML.

Browser performance budgets live in [`../tests/baselines/ui-performance.json`](../tests/baselines/ui-performance.json). The performance test records startup, settings open and close, section-change p95, long tasks, and uncaught errors. Change a budget only after repeated measurements show that the old limit no longer represents the supported local environment.

On Windows, use `python` instead of `python3` if `python3` is not available in your shell.

## Playwright

Use Playwright for UI-facing changes when the app can be exercised through the browser dev server.

```bash
# Owns isolated renderer servers and adapts the frozen smoke URLs.
python3 tests/OnePyFone.py --suite ui,state

# Owns a real Rust backend, browser checks, and a persistence restart.
npm run test:session:owned
```

For interactive inspection, start your own
`npm run dev:session -- --synthetic-seed` and use its private access link.
See [Browser dev sessions](DEV_SESSIONS.md). A mock preview cannot establish
backend or native behavior.

[`../tests/smoke/`](../tests/smoke/) is a frozen contract. Do not edit those files unless the user explicitly asks. Fix app code to satisfy them. Add real-session browser coverage in [`../tests/browser/`](../tests/browser/).
Existing mock integration checks live in [`../tests/integration/`](../tests/integration/).

## Rust Tests

LAN sync has a focused deterministic gate:

```bash
npm run test:sync
```

It includes two virtual devices with separate temporary databases, actual
loopback TCP/TLS connections, code-based pairing, multi-batch transfers,
database restart, interrupted transfer recovery, and a three-device relay
case. These tests also run in the ordinary Rust suite. See
[`lan-sync.md`](lan-sync.md) for coverage, boundaries, and the physical
two-device checklist.

```bash
npm run test:rust
cargo test --manifest-path src-tauri/Cargo.toml <test_name>
```

Rust tests cover pure logic, provider error classification, prompt assembly, pipeline fixtures, SQLite behavior, context decisions, snippets, dictionary behavior, and data validation.

Prompt contracts are data-driven in [`../tests/fixtures/prompt-regressions.json`](../tests/fixtures/prompt-regressions.json). Deterministic cases inspect the assembled prompt. Live cases call the configured cleanup model and check meaning, instruction boundaries, forbidden behavior, and output limits without exact-string matching.

## Privacy Rules For Tests

- Do not print API keys.
- Do not print clipboard contents.
- Do not include real dictated text in fixtures or output.
- Do not commit screenshots with private text.
- Live provider tests read the provider and model from Verenu settings and the credential through Verenu's Rust credential module. They never print the key or full model output.
- CI may use provider-key environment secrets because OS credential stores are unavailable on hosted Linux runners. Local environment-key fallback stays disabled unless `VERENU_ALLOW_ENV_CREDENTIALS=1` is set explicitly.
- Live provider tests must skip cleanly when credentials, provider support, or optional fixtures are unavailable.

## CI

GitHub Actions currently run:

- Frontend type-check and build.
- npm and Rust dependency audits.
- Rust Clippy and Rust tests on Windows and macOS.
- OnePyFone fast profile with JSON and JUnit reports.
- Extended live/native profiles on schedule or manual dispatch.
- Manual installer builds through `workflow_dispatch`.

PR checks also run Android Kotlin compilation/JVM tests, native WebViews on all
three desktop platforms, and workflow/test-infrastructure contracts with
actionlint. Shared Linux regression steps include Clippy, all owned-session
suites, desktop/phone visual comparisons, and WCAG accessibility scans. Known
contrast violations are recorded in reviewed JSON baselines; structural
violations or changed contrast results fail. These baselines are regression
protection, not a claim that the existing UI meets every WCAG criterion. The
nightly publisher waits for those regression checks and Windows/macOS Rust
checks against its exact versioned source snapshot. Live jobs without provider
credentials report incomplete and fail instead of claiming provider success.

`node scripts/ci/merge-rules.mjs` checks the deployed required status checks.
`--apply` installs the master-only CI ruleset without editing other rulesets.
The new aggregate `CI required` check can be required after the updated workflow
has landed; the deployed ruleset initially requires the already-existing checks.

## Related Docs

<p align="center">
  <a href="ARCHITECTURE.md"><img alt="Architecture" src="https://img.shields.io/badge/Architecture-Overview-5b554a"></a>
  <a href="CONTRIBUTING.md"><img alt="Contributing" src="https://img.shields.io/badge/Contributing-Guide-c44632"></a>
  <a href="RELEASE.md"><img alt="Release Process" src="https://img.shields.io/badge/Release-Process-7e7266"></a>
  <a href="README.md"><img alt="Docs Index" src="https://img.shields.io/badge/Docs-Index-2b2422"></a>
</p>
