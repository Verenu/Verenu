# Agent-guided verification

The completion gate selects checks from changes relative to `master` and explicit
task requirements. Exit codes are 0 for `verified`, 1 for `failed`, and 2 for
`incomplete`. Missing capabilities are gaps, never successful tests.

```bash
npm run verify:task -- --task test-results/task.json --evidence test-results/outcomes.json
npm run verify:task -- --task test-results/task.json --require native,pipeline
npm run verify:task -- --inspect-only --task test-results/task.json
```

`--inspect-only` evaluates evidence without launching checks. `--base REF` changes
the comparison branch. Explicit requirements extend automatic selection. A
private task file contains acceptance criteria:

```json
{
  "require": ["session"],
  "acceptance": [{"id": "persist", "expected": "An edited Context survives backend restart"}]
}
```

The gate runs type checks/build, units, Rust tests/Clippy, renderer contracts,
real sessions, live pipeline checks, and native WebView tests when required.
Migration checks run existing database tests. Add regression coverage for a
specific migration and verify preservation of existing data; generic database
tests alone do not prove newly requested migration behavior.

Identity hashes tracked and untracked non-ignored source contents, paths,
deletions, and modes. Reports include that fingerprint and the worktree.
Commit SHA and dirty flags do not identify uncommitted changes. Further edits
invalidate evidence. Rust workers embed the fingerprint at compilation.

Observed outcomes supplied through `--evidence` use this shape:

```json
{
  "records": [{
    "category": "acceptance",
    "criterion": "persist",
    "status": "passed",
    "worktree": "/absolute/current/worktree",
    "fingerprint": "current-source-sha256",
    "observed": "The same Context ID and edited name remained after restarting Rust",
    "artifacts": [{"path": "/absolute/evidence.png", "sha256": "file-sha256"}]
  }]
}
```

UI changes require an `inspection` record with `inspected: true`, observed states,
and screenshot artifacts. Native OS changes require `native-integration` evidence
from relevant disposable targets and affected platforms. WebView evidence cannot
satisfy that category. Supplied artifact hashes are checked. Agents must still
inspect images and describe observations accurately; a report cannot prove that
someone looked at a file.

Native evidence must include `platform` (`linux`, `darwin`, `win32`, or `android`)
and a `scope` array naming the capabilities actually exercised. A task can add
`native: [{"platform":"linux","scope":"shortcuts"}]` requirements. The gate
also derives capability requirements from hotkey, injection, permission, audio,
and window-integration source paths. Focused-text evidence cannot satisfy
shortcut activation or clipboard restoration. Manual native records are kept
alongside the automatic fixture evidence. Skipped or flaky acceptance outcomes
remain incomplete even when another record passes.

Workflow edits require `npm run test:ci`, which checks release and merge gates
and runs checksum-verified actionlint. Android edits require `npm run test:android`
with an Android SDK, NDK, JDK, and Rust Android target. This builds a real debug
APK and runs the JVM regression tests. It does not establish packaged inference
runtimes or device behavior.

## Owned sessions

```bash
npm run test:session:owned
npm run test:session:owned -- --live
npm run dev:fixtures
npm run dev:session -- --synthetic-seed
```

The owned suite generates WAVs, starts Rust, runs transport and Playwright flows,
restarts the backend with its isolated database, verifies persistence, and stops
only its own processes. Default runs use public synthetic settings without
installed customization. `--live` uses an isolated installed-configuration
snapshot and native credentials. That snapshot and all artifacts remain private.

Every `tests/dev-session/*.test.mjs` file runs sequentially. Structured reports
name every executed case and identify missing files. The optional live corpus is
the only permitted skip in a deterministic session. Browser verification requires
both desktop and phone projects and rejects any skipped case. Use
`npm run test:session:owned -- --update-snapshots` only to deliberately regenerate
visual baselines, inspect the images, and then rerun without that flag. CI never
updates baselines. The renderer registry also rejects unregistered smoke or
integration scripts; manual-only exclusions need a recorded reason.

The live corpus checks plain speech, punctuation, correction, vocabulary,
snippets, and longer speech against new history IDs, exact stored output, and
matching events. Existing deterministic Rust pipeline fixtures cover provider
errors, fallback, cleanup failure, caching, and snippets. Live calls are opt-in
because they incur provider cost.

New browser tests in `tests/browser/` use a shared real-session fixture and
desktop/phone projects with retries disabled. Failures retain private traces,
screenshots, and videos. Traces may contain ephemeral tokens and customization.
Never publish raw traces/access files. CI uploads structured reports by default.
The owned-session summary records only project, repository-relative spec path,
line, static test title, outcome, and retry count for each Playwright case. It
omits raw browser errors and reporter attachments. Failed attempts also retain
fixed error classifications, allowlisted assertion matcher names, validated source
locations, and JSON pointers to trace attachments in the private `playwright.json`
report. Assertion values, selectors, call logs, stack traces, attachment paths and
bodies are never copied. Unknown errors get a generic withheld-details summary.
Failed-attempt diagnostics survive a passing retry; retries still fail the gate.

Trace references are metadata only, not downloadable CI artifacts. The shared
regression action uploads only `tests/reports/` and deletes its isolated session
home on exit, including raw traces. References identify reporter-listed attachments
and do not prove file availability. Reproduce locally to inspect private traces. Do not
broaden uploads to recover raw traces, screenshots, videos, or session access files.

Owned Node regression failures include the static test title, repository-relative
test file, safe error type/code, and assertion source line when available. They
omit assertion values, error messages, console output, and private session paths.

Frozen smoke tests remain unchanged. A process-local adapter redirects their
fixed localhost URLs and screenshots to owned servers/directories. It changes
no assertions, responses, or app behavior. `--fresh-server` is a compatibility
flag; default servers are always fresh. `--no-server` requires an explicit
`--test-url` for your own renderer. Never point mock tests at a live worker.

## Native verification

```bash
npm run test:native:webview
npm run test:native:fixtures
npm run test:native:prerequisites
```

An isolated client connects to the embedded WebDriver in a native worker. The
WebDriver executes real native IPC. Tests check the actual WebView, Context reload
persistence, window geometry, and a screenshot. They do not prove external
insertion or shortcuts. `native-testing` is opt-in and forbidden in release
builds. Production capabilities contain no WebDriver permissions.

The Linux fixture starts an owned GTK entry and runs the existing AT-SPI test
against its PID. Unsupported platforms/missing displays return `incomplete`.
Shortcut and clipboard checks still need dedicated desktop targets. Record
Windows, macOS, and Hyprland/Wayland results separately. Xvfb cannot establish
Hyprland portal behavior.

## Evaluate agents

```bash
npm run test:agent-evals -- --list
npm run test:agent-evals -- --agent-command '["agent-cli", "{prompt}"]' --trials 3
```

The pilot seeds Rust skip parsing, optional failure exit codes, and retry
evidence bugs. Each trial gets a disposable repository snapshot. The independent
grader remains outside that snapshot, confirms the seed fails, and checks
outcomes independently of agent-edited tests.

Results record correctness, false completion claims, missing reports, duration,
reported checks/skips, and reported provider cost. Cost/activity are
agent-reported. This pilot measures test-infrastructure fixes, not general
Verenu reliability. Extend it with independently graded Context, dictation, and
native scenarios before comparing overall agent quality. Private logs and
snapshots stay in ignored `test-results/`. No agent runs without a command.
