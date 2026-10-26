# Agent and script control

Verenu provides an opt-in, versioned local API backed by the running native
app. Settings and Context writes use the same validation and runtime effects as
the UI. Agents never need to write live SQLite or read native credentials.

Requires Node 20+ for the dependency-free CLI. From a source checkout run
`npm run agent -- --help`. The bundled `agent-skill/` resource also contains
`scripts/verenu-agent.mjs`; run that script directly with Node. Resource locations
follow Tauri's platform resource directory (inside the app on macOS and beside
the installed resources on Windows/Linux).

## Enable access

Close Verenu normally, then start it with a private grant. For example on Linux:

```bash
export VERENU_AGENT_PORT=17439
export VERENU_AGENT_TOKEN="$(openssl rand -hex 32)"
verenu &
node Agent-Skills/verenu-control/scripts/verenu-agent.mjs describe
```

Use the executable inside the app bundle on macOS. PowerShell examples are in
the CLI's `--help`. Keep the variables in the invoking shell; do not print the
token, put it in a command argument, or commit it. It grants settings/Context
writes and explicit transcript reads. Restarting without these variables revokes
access. Invalid configuration or an occupied port fails startup; no port fallback
or automatic credential discovery occurs. The API is off in ordinary launches.

The API binds only `127.0.0.1`, requires a 64-character random hex bearer token,
rejects browser Origin/Fetch-Metadata requests, and provides no CORS access.
Other processes with the grant can act as the user. Loopback is not a substitute
for keeping the grant private. No credential, arbitrary SQL, shell, microphone
capture, or text-injection endpoint exists.

## Discover and install the skill

```bash
npm run agent -- harnesses
npm run agent -- install-skill
```

Interactive installation detects existing Codex, Claude, Cursor, Grok and
OpenCode configuration directories, asks whether to install permanently, then
asks which detected harnesses to use. It copies only the selected skill folders;
it never edits generated global instructions. Declining or making no selection
does nothing. Directory detection does not guarantee that a harness version
supports skills. Noninteractive installation requires both explicit switches:

```bash
npm run agent -- install-skill --permanent --harness codex
```

Existing destinations are refused unless `--replace` is explicit. Updates are
explicit reinstallation, not an automatic startup action. Installed scripts still
need the running app's private grant; installation does not enable the API.

## Supported operations

Run `describe` for the authoritative registry, argument names and writable keys.
Native command arguments use camelCase; dotted history operations use snake_case.
Unknown fields and unsupported commands fail. Dates are UTC `YYYY-MM-DD` with
inclusive start and exclusive end.

```bash
npm run agent -- get_contexts
npm run agent -- save_setting '{"key":"mute_audio","value":true}'
npm run agent -- history.report '{"start":"2026-01-01","end":"2026-04-01"}'
npm run agent -- diagnostics.top '{"start":"2026-01-01","end":"2026-04-01"}'
```

Use `--stdin` instead of a JSON argument for private instructions. The protocol
is `POST /v1/invoke`, `Authorization: Bearer <grant>`, with
`{"command":"...","args":{...}}`. Success is `{"version":1,"result":...}`;
failures have a non-2xx status and `error`. Requests are limited to 64 KiB, one
active request and 60 seconds. After a timeout, read state before retrying a
mutation: cancellation does not guarantee a native operation was undone.

- Read settings and microphones; write the discovered allowlist of audio,
  cleanup, caps, spacing, provider/model, language, prompt and history settings.
- Create, duplicate, rename, pin, color, edit or delete Context groups; inspect
  stats, dictionary and snippets; assign/remove apps and website targets.
  Reassignment moves a target. `update_context_settings` replaces its settings:
  read first and preserve fields you are not changing. Everywhere semantics
  remain owned by the existing native handlers. `create_context` and
  `update_context_settings` accept the optional `pasteInChunks` preference.
- Read `diagnostics.snapshot`, lint current prompt templates using `prompts.lint`,
  or explicitly invoke `test_cleanup_prompt` with the existing saved provider
  credentials. That last command makes a live provider request and may cost
  money. Linting alone does not prove provider behavior.
- Read `history.report`, bounded `history.page`, and `diagnostics.top`; configure
  journal retention using `diagnostics.retention`.

Read back mutations. Open/reload the relevant UI view to observe external edits;
already-mounted UI state is not automatically refreshed by this interface.

## Historical coverage and privacy

Existing transcription history already stores raw and cleaned text, dates,
provider, app and Context ID. Its existing retention remains independent:
`history_retention` accepts `7 days`, `30 days`, `90 days`, or `Forever`.
Shortening retention immediately deletes older history. Do not do so merely to
test an agent workflow on someone's real data.

`history.report` returns retained counts/dates, top ten app/Context counts and
capitalization-only, whitespace-only, unchanged and other cleanup-change signals.
Signals are not confirmed mistakes or user corrections. Classification scans at
most 100,000 rows and reports truncation; total counts cover the full range.
`history.page` returns at most 500 rows per request, with ascending-ID cursors.
Text is absent unless `include_text:true` is explicit. Authorized text output is
sensitive: keep it local, out of public reports, traces and screenshots.

The new failure journal is **off by default**. Explain retention before enabling:

```bash
npm run agent -- diagnostics.retention '{"days":90}'
```

It stores only daily UTC counts, fixed subsystem categories and opaque
diagnostic fingerprints, for 0..365 calendar days including today. Subsystem
labels include a coarse fixed category (rate-limit, authorization, timeout,
connection, response-format or unclassified)
derived from error keywords. These are investigation hints, not diagnoses.
It stores no error messages, dictated text, prompts,
paths, model names or credentials. Setting days to 0 disables and clears it;
shortening retention prunes immediately. Startup, periodic maintenance and reads
also prune. The additive schema leaves existing transcript rows/settings intact.
Deletion is logical SQLite deletion, not secure erasure of backups or disk blocks.

The journal has a 50,000 daily-group limit (oldest groups may be evicted), a
512-event nonblocking queue, and a process-local dropped-event counter. Abrupt
shutdown can lose queued events. Counts are recorded failure events, not unique
dictations. Warning-only failures may not be recorded. Current-session diagnostic
details can help interpret a matching fingerprint; old messages are not retained
by this journal. Ordinary redacted logs retain their existing 30-day/256 MiB
policy. There is no historical backfill or reconstruction of deleted history,
original composed prompts, past settings, audio, or final manual corrections.
Report gaps when investigating weeks or months; never manufacture a top ten.

## Audio-route example

`scripts/audio-policy.mjs` supplies a one-shot Linux PipeWire/PulseAudio example:

```bash
node Agent-Skills/verenu-control/scripts/audio-policy.mjs --mute-output speakers --unmute-output headphones
node Agent-Skills/verenu-control/scripts/audio-policy.mjs --mute-output speakers --unmute-output headphones --apply
```

It uses `pactl`'s default output and active port. Supply case-insensitive literal
substrings matching your output names or ports; there are no built-in device
rules. An unmute match takes priority, and an unmatched route leaves the setting
unchanged. Default mode is a dry run. `--apply` uses validated
`save_setting` and reads the result back. It installs no background watcher and
does not claim that a saved microphone selection proves the live output route.
Other platforms can feed their supported device inventory into the same native
setting API; hardware routing must be verified on the actual machine.
