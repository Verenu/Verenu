# Verenu with the Lust fork

This optional Linux x86_64 workflow checks real Verenu modules with our Lust fork,
runs their compatible tests, and compares them with rustc. Verenu's desktop app
continues to build with Cargo. This target does not include Tauri, provider calls,
async code, native integration, or the complete dictation pipeline.

The isolated target avoids extracting an application crate solely for an
experimental compiler. That alternative would change visibility, platform
gating, and app call sites and require verification across the shipping
platforms. Keeping the boundary here allows compiler compatibility to advance
without changing those contracts.

## Run

Prerequisites: Node.js, Git, Cargo/rustc, Bash, Make, a C compiler, and libc
development files. Setup needs network access and builds the Lust fork and its
pinned TinyCC against the host. This avoids the Debian-specific CRT search paths
in the upstream Linux release archive on Arch Linux.

```sh
node tools/lust/run.mjs setup
node tools/lust/run.mjs check
node tools/lust/run.mjs test
node tools/lust/run.mjs bench
node tools/lust/run.mjs full-check
node --test tools/lust/run.test.mjs
```

`toolchain.json` pins [MONKE2525E/lust-lang](https://github.com/MONKE2525E/lust-lang/tree/verenu-support)
at commit `256c964fd72ea3e2c99798fbaef34d6bfa813fc1`, based on upstream
`91dce84cf1568fb39954b2717a7a3dddedce82d4`. The fork adds `Chars::as_str`
with the original string's lifetime and differential runtime and borrow tests.
It also adds a tested `log` 0.4.34 source adapter and correct Cargo target names
in `module_path!()` metadata.
The full desktop backend still requires substantial compiler development.
To use an existing installation instead, set `LUST_BIN` to its
`bin/lust` or `target/release/lust` executable. The matching `lust-core` and
runtime files must remain alongside it in the upstream layout. Missing or
incompatible compilers fail; the workflow never substitutes rustc for Lust.

Generated sources, toolchain builds, isolated compiler caches, binaries, and
JSON reports live under ignored `test-results/lust/`. Each invocation prints its
report path. Reports contain compiler versions, source SHA-256 hashes, command
arguments, diagnostics, exit codes, and timings. The toolchain can take several
minutes to build. Keep credentials and private dictation data out of workloads.
Rerunning setup reuses the pinned checkout and build output. It refuses to
overwrite modified sources or a checkout at another commit. Each pinned revision
has a separate checkout. The workflow requires setup or an explicit `LUST_BIN`;
it does not select an arbitrary compiler from `PATH`.

## Full backend compatibility probe

`full-check` attempts the actual library with its complete dependency graph,
then the library and desktop binary with an isolated edition-2024 manifest.
The source and native dependencies stay intact. It also checks small async and
native-FFI programs with rustc and Lust to distinguish language gaps from
application dependency errors. Every command and diagnostic goes into the JSON
report. This is a compatibility probe: any failed backend attempt exits nonzero.
It does not switch the application's compiler or invoke Lust's Cargo release
fallback. Cargo may prepare build scripts and procedural macros during Lust's
dependency discovery; that preparation is not successful Lust compilation.

The tested fork rejects the real edition-2021 manifest with L0530. With only
the copied edition changed, both targets pass the previous `log::warn!` expansion
blocker and reject `serde_json::json!` with L0802. The
standalone probes are valid Rust; Lust rejects async with L0405 and native FFI
with L0810. Logging support therefore does not compile the desktop. Verenu's
provider requests, portal hotkeys, Tauri command macros, SQLite, audio, and
native integrations require broader compiler and dependency support.

Current upstream also explicitly excludes async, unsafe code, and custom
procedural macros; see its [supported subset](https://github.com/t3dotgg/lust-lang/blob/91dce84cf1568fb39954b2717a7a3dddedce82d4/README.md).
Making the complete desktop backend work requires compiler development beyond
this harness. The shipping Cargo manifest remains unchanged.

Full-backend attempts share this worktree's ignored
`test-results/lust/full-backend-target/` Cargo output to avoid rebuilding native
build tools on every invocation. Each attempt still has its own compiler cache,
manifest overlay, lockfile copy, and diagnostics.

## Coverage

`targets.json` is the module inventory. Missing source paths fail immediately.
The shipping implementations and their assertions are not changed.

| Module | Check | Existing tests |
| --- | --- | --- |
| Device name matching | Yes | 8 |
| Digital silence and mute debouncing | Yes | 5 |
| Hotkey gesture state machine | Yes | 25 |
| Hotkey chord parsing | Yes | 2 |
| Clipboard phrase handling | Yes | Blocked: Lust L0809 on a long assertion |
| Insertion context policy | Yes | Blocked: test depends on the app's skills catalog |
| Text utilities | Yes | 22 |
| Spoken text rules | Blocked: external Unicode dependency | Blocked |

`check` checks production code from seven files through `#[path]` imports, then
checks the same root with rustc, using edition 2024 for both. Conditional code
follows the host platform. Digital silence and gesture logic serve Windows in
the application; running their tests on Linux does not verify OS integration.

`test` runs all 62 tests in the five compatible modules under both compilers and
compares names and results. Zero tests, missing tests, ignored tests, failures,
and mismatches fail the command. It also confirms both compilers reject a
deliberately invalid function call.

The pinned fork discovers nested test modules. Both compilers receive the same
source bytes, including the original test functions and assertions, without
dispatch wrappers. No fake dependencies or replacement implementations are used.
Blocked modules and tests appear explicitly in console output and reports.

## Interpret the benchmark

The benchmark measures two separate operations, five samples per compiler in
alternating order, after warmup:

- A body edit to a private snapshot of the production device matching module,
  in a target that imports the checked production modules. Each edit adds a different
  always-true integer comparison to its exact-name branch. Both compilers see
  the same snapshot; the live worktree is never edited.
  Lust uses its own warm daemon; rustc uses incremental metadata checking.
- A development binary performing 200,000 calls each to real device matching
  and context classification logic on synthetic strings. Both binaries must
  produce the same checksum on every run. Lust uses TinyCC; rustc uses opt level
  zero. This does not compare optimized release binaries.

The driver prints median, minimum, and maximum wall time in milliseconds. It
records CPU, load average, governor when available, and raw samples. It also
checks an invalid body edit after timing to ensure the warm check invalidates.
Each daemon has a unique socket, a short idle timeout, and its socket is removed
at completion to request shutdown. Caches and rustc incremental state are local
to the invocation.

These are core development measurements, not desktop startup or dictation
latency. A complete-app speedup cannot be inferred from them. Lust's
`--release` forwards to Cargo or rustc, so this driver deliberately avoids that
flag. Use Cargo for optimized production builds. Profile separately before
attributing a timing difference to a particular function or adopting a compiler
for runtime performance.
