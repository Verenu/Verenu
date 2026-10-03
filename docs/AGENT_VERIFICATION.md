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

The live corpus checks plain speech, punctuation, correction, vocabulary,
snippets, and longer speech against new history IDs, exact stored output, and
matching events. Existing deterministic Rust pipeline fixtures cover provider
errors, fallback, cleanup failure, caching, and snippets. Live calls are opt-in
because they incur provider cost.

New browser tests in `tests/browser/` use a shared real-session fixture and
desktop/phone projects with retries disabled. Failures retain private traces,
screenshots, and videos. Traces may contain ephemeral tokens and customization.
Never publish raw traces/access files. CI uploads structured reports by default.

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

WebdriverIO connects to the embedded driver in an isolated native worker. The
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
