# Agent and script control

Verenu provides an opt-in, versioned local API backed by the running native
app. Settings and Context writes use the same validation and runtime effects as
the UI. Agents never need to write live SQLite or read native credentials.

Requires Node 20+ for the dependency-free CLI. From a source checkout run
`npm run agent -- --help`. The bundled `agent-skill/` resource also contains
`scripts/verenu-agent.mjs`; run that script directly with Node. Resource locations
follow Tauri's platform resource directory (inside the app on macOS and beside
the installed resources on Windows/Linux).

## Connect an agent

Start Verenu normally. The agent creates a private session directory, sets
`VERENU_AGENT_GRANT_FILE` to a new file there, and runs the CLI `connect`
command. The CLI returns a code; compare it with the request in Verenu before
approving. The agent runs `status` after approval and uses the same grant file
for later commands. Grants expire after one hour or when Verenu closes. Revoke
all agents from Settings > Privacy, or use `disconnect` for the current agent.
The grant file is private: do not print its contents or send it in chat.

The code identifies the request, not the executable or its publisher. Approve
only when it matches the code from the agent you intended to connect. Local
processes running as the same OS user may be able to inspect private files or
modify Verenu's data directly; this is not a sandbox against such processes.

## Scripted use

Open Verenu normally, then request access from the bundled CLI:

```bash
session_dir="$(mktemp -d)"
export VERENU_AGENT_GRANT_FILE="$session_dir/grant.json"
node Agent-Skills/verenu-control/scripts/verenu-agent.mjs connect
# Compare the returned code with Verenu and approve it before continuing.
node Agent-Skills/verenu-control/scripts/verenu-agent.mjs describe
```

The agent performs this setup; the user does not need to launch a terminal. Use
the executable inside the app bundle on macOS. For an explicit environment grant,
PowerShell examples are in the CLI's `--help`. The CLI or agent must inherit the
same variables as the app when using that explicit grant;
exporting them in a different terminal does not give an existing agent session
access. Keep the variables in the invoking shell; do not print the
token, put it in a command argument, or commit it. The approval dialog selects
permissions for each agent, including separate transcript, credential, and
destructive permissions. Grants expire after one hour or when Verenu closes.
The explicit environment grant has all permissions. Restarting without these variables revokes
access. Invalid configuration or an occupied port fails startup; no port fallback
or automatic credential discovery occurs for environment grants. Ordinary launches
expose the approval, status, and revocation endpoints; data commands require an
approved grant.

The API binds only `127.0.0.1`, requires a 64-character random hex bearer token,
rejects browser Origin/Fetch-Metadata requests, and provides no CORS access.
Other processes with the grant can act as the user. Loopback is not a substitute
for keeping the grant private. The environment-based grant is not a security
boundary against malware running as the same operating-system user: such a
process may be able to inspect the app's environment or modify its data directly.
Use OS account isolation for that threat. No credential-read, arbitrary SQL,
shell, microphone capture, or text-injection endpoint exists.

## Discover and install the skill

The app also bundles a small `agent-install-skill/` guide for agents handling
Verenu requests. It explains how to use the control skill in place or, with the
user's agreement, save it for the current harness and optionally selected
other harnesses. On ordinary desktop startup, Verenu registers this discovery-only
guide in detected harnesses. It does not overwrite existing skills, and it
does not reinstall a guide the user removed. A harness detected later is
registered on a subsequent startup. This does not enable the API or install
the control skill.

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

`library.usage` reports locally recorded mechanical vocabulary substitutions
and snippet expansions, with a tracking start and last-match times. Tracking
begins with the upgrade and older daily buckets are pruned with history
retention. It does not capture use on other devices or influence on speech
recognition or cleanup prompts. Recording is best effort; a missing match is
not proof that an item is unused. Dictionary correction counts measure learning,
not these matches. Snippet expansion counts include attempts that may not finish.

`audit.page` lists recent agent write attempts, their operation, numeric target
when available, and status. It stores no supplied text, values or tokens. It
shows the last 30 days and removes older records on the next agent write.
Each approval has an opaque identifier shared by its audit entries; callers
using the same grant are indistinguishable, as are environment-grant callers.
Audit writes and application changes
are separate transactions: a pending entry or interrupted operation has an
uncertain outcome, and even a failed operation can require a state read before
retrying.

```bash
npm run agent -- get_contexts
npm run agent -- get_setting '{"key":"mute_audio"}'
npm run agent -- save_setting '{"key":"mute_audio","value":true,"expected":{"mute_audio":false}}'
npm run agent -- history.report '{"start":"2026-01-01","end":"2026-04-01"}'
npm run agent -- diagnostics.top '{"start":"2026-01-01","end":"2026-04-01"}'
```

Use `--stdin` instead of a JSON argument for private instructions. The protocol
is `POST /v1/invoke`, `Authorization: Bearer <grant>`, with
`{"command":"...","args":{...}}`. Success is `{"version":1,"result":...}`;
failures have a non-2xx status and `error`. Requests are limited to 64 KiB, one
active request and 60 seconds. After a timeout, read state before retrying a
mutation: cancellation does not guarantee a native operation was undone.

- Read settings and microphones; write the discovered allowlist of validated
  user settings. Use `save_hotkey` and `set_autostart` for their native effects;
  pass the raw `hotkey` or `autostart_enabled` value read with `get_setting` as
  `expected` (including `null` if unset). Stale values reject the change.
  `save_setting` requires the current raw values in `expected`; when changing
  `contextual_formatting_enabled`, include `contextual_caps_enabled` and
  `auto_spacing_enabled`. A stale value rejects the complete write.
- Create, edit, delete and assign dictionary entries and snippets; create,
  assign and delete sub-apps. Inspect `describe` for the complete command list.
  Entry edits require the original editable fields in `expected`; reread and
  reconcile after a conflict.
- `credentials.save` and `credentials.delete` manage saved provider keys through
  the native credential store; `get_api_key_status` reports presence, never key
  values. Pass secrets with `--stdin`, not command arguments, and do not log them.
  Sync pairing and device-management commands can connect another device and
  should be invoked only when the user has requested that operation. The same
  applies to `t3.connect`, whose pairing link must be supplied via private stdin.
  Use `delete_custom_provider` for removal so selected models are repaired.
- Create, duplicate, rename, pin, color, edit or delete Context groups; inspect
  stats, dictionary and snippets; assign/remove apps and website targets.
  Reassignment moves a target. Use `context.patch_settings` for a partial edit
  with `expected` set to the original Context from `get_contexts`.
  Omitted fields remain unchanged; null clears a nullable setting. For a whole
  Context replacement, use `save_context_if_unchanged` with its original and
  updated records, including changes to T3 skill mentions. Pin or unpin with
  `context.set_pinned`, passing the original Context as `expected`. Everywhere semantics
  remain owned by the existing native handlers. `create_context` and
  `context.patch_settings` accept the optional `pasteInChunks` preference.
  The API also exposes the vocabulary and snippet inventories, their explicit
  Context assignments, and the native assignment setters. Effective Context
  inventories include inherited Everywhere items.
  Agents can create, edit and remove vocabulary and snippets using the native
  library commands. `remove_dictionary_entry` without `contextId` deletes the
  term and its corrections globally; with `contextId` it removes that Context's
  assignment and corrections. `edit_dictionary_entry` replaces the selected
  Context's correction patterns and clears related auto-learn evidence; include
  all patterns that should remain. A scoped edit cannot rename a term assigned
  to another Context. `rename_dictionary_entry_global` requires `expectedTerm`
  from the original entry and deliberately renames a
  shared term in every Context without replacing its corrections; it clears
  related learning evidence and requires destructive access. Editing or deleting a snippet affects every
  Context that uses it. For `edit_snippet`, pass `expected` with the original
  `trigger`, `expansion`, and `instructions` to reject a concurrent edit.
  These direct operations have no built-in undo. Check the item
  and its assignments before changing it. Correction counts are not usage
  counts, and zero does not establish that an item is unused.
- For bulk global deletes, `library.items` returns item UUIDs.
  `library.preview_delete` accepts up to 100 `{kind, uuid}` items and returns
  their corrections, local recorded matches, explicit Context assignments, inheritance effects and a
  basis. `library.apply_delete` accepts the same items, the basis and a unique
  UUID `request_id`; it rejects a changed preview and performs the deletion in
  one transaction. Retrying the same request returns its receipt. Use
  `library.undo_delete` with the request ID to restore the whole batch within
  30 days. Restore fails atomically if an entry, correction or Context now
  conflicts. It preserves sync UUIDs but does not restore purged learning
  evidence. The local recovery record contains the deleted content; expired
  records are removed on the next bulk deletion or app start. It is not synced
  or included in backups. Counts are not proof that an entry is unused.
- Read `diagnostics.snapshot`, lint current prompt templates using `prompts.lint`,
  or explicitly invoke `test_cleanup_prompt` with the existing saved provider
  credentials. That last command makes a live provider request and may cost
  money. Linting alone does not prove provider behavior.
- Read `history.report`, metadata-only `history.usage`, bounded `history.page`, and `diagnostics.top`; configure
  journal retention using `diagnostics.retention`.

Read back mutations and reconcile any reported conflicts before retrying.

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
