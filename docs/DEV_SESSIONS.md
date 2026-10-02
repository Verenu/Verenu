# Browser dev sessions

A dev session runs the Svelte UI in a browser and the real Rust backend from
the current worktree. Commands use the existing Tauri handlers. Uploaded audio
uses the production dictation pipeline and returns text without pasting into
other applications.

## Start and share

```bash
npm install
npm run dev:fixtures
npm run dev:session -- --share
```

The launcher prints the session directory and URLs. Open `localAccessUrl` from
its private `access.json` in the collaborative browser. The URL fragment carries
an ephemeral session token; the page removes it and keeps it in session storage.
The UI requires the live backend and never falls back to preview mocks.

`--share` creates a private Tailscale HTTPS listener on a separate port. Use
`shareAccessUrl`, or the panel's **Copy phone access link**, for a phone signed
into the same tailnet. Keep access links private. HTTPS supports browser mic
permissions. Tailscale must already be running with HTTPS enabled and Serve
permission. The launcher tries scoped passwordless sudo only if Serve denies
access; it does not change device operator settings or use Funnel.

Each thread uses its T3 worktree. Each launch owns separate ports, a database,
settings, an authentication token, fixtures, and a session manifest. Build output
is confined to that worktree. `npm run dev:session -- --list` lists manifests.
Stop your launcher with Ctrl-C. Shutdown stops its children and removes only
its Tailscale listener. Session data remains for inspection; T3 worktree cleanup
does not delete it. After a forced kill, inspect ownership before removing a
stale lock or listener. Never reset all Tailscale Serve routes.

Restart after changing Rust code. The launcher deliberately disables backend
watching. Vite still reloads frontend changes. Reusing `--id NAME` retains that
session's data after a normal shutdown; use a fresh ID for a fresh snapshot.

## Data and credentials

The Rust worker takes a SQLite backup of the installed app database and copies
settings before opening its isolated copy. It does not modify the source.
Contexts, targets, vocabulary, and snippets remain. By default it removes
transcript history, transcript-derived state, and sync identity from the copy.
`--private-history` explicitly retains history. Treat copied customization and
all screenshots as private even without history.

The worker reads existing provider credentials through the native credential
store. Keys stay on the host and are removed from copied settings. The bridge
does not expose key values or allow credential writes. Sync, analytics,
automatic vocabulary learning, global hotkeys, clipboard injection, and native
tray startup are disabled. Desktop microphone access requires `--host-mic` and
a cross-process lease, so two sessions cannot capture the host mic together.

Use `--seed-dir PATH` to choose a different installed data directory. The
worker still uses the current host's native credentials. Startup requires the
normal Tauri toolchain and a working desktop display; this is not a standalone
headless server. The session bridge is a debug-only Cargo feature.

## Test the changed behavior

Open **Dev tests** to select a context or simulated process/domain, run a fixture,
record browser audio, import a clip, inspect the returned transcript, and read
redacted backend logs. Optional desktop mic capture uses the production capture
path but returns text through events. The default limit is 30 dictation/cleanup
attempts per worker. `--max-runs N` adjusts it; this is an attempt limit, not a
currency budget. Provider calls use your account.

`npm run dev:fixtures` creates public synthetic speech and silence in
`~/.cache/verenu/test-audio`. It requires `espeak-ng` and `ffmpeg`. The manifest
in `tests/fixtures/dev-audio.json` covers plain speech, punctuation, correction,
vocabulary, snippets, and longer speech. Clips are generated offline. You can
import an ElevenLabs-generated or other synthetic clip in the panel; browser
decoding converts it to 16 kHz mono PCM. Never use private dictation as a fixture.
`--fixtures PATH` supplies a different directory of WAV clips.

```bash
VERENU_SESSION_ACCESS_FILE=/path/to/session/access.json npm run test:dev-session
VERENU_SESSION_ACCESS_FILE=/path/to/session/access.json VERENU_DEV_REQUIRE_LIVE=1 npm run test:dev-session
```

The first command checks authentication, blocked native commands, real Context
CRUD, audio validation, silence gates, backend events, desktop/phone layouts,
and refusal to use mocks without authentication. The second also requires real
synthetic dictation and saved history through the configured providers. Reports
go to the private session directory as `verification.json`. A skipped live test
does not prove provider behavior. Ordinary unit tests require no credentials.

Agents must inspect their changed flow in the collaborative browser, add
regression coverage for changed behavior, and fix failures before handoff.
Check desktop and phone widths when UI changes. For dictation, run appropriate
synthetic clips and inspect both events and history. Report the worktree/branch,
checks performed, skipped checks, and private phone link when sharing a session.
Do not ask the user to perform browser checks that the agent can perform itself.

Browser checks cannot verify global shortcuts, native permissions, OS text
injection, focus restoration, or desktop window behavior. Those still need
native verification on the affected platforms. Credential editing and native
actions rejected by the bridge are not browser-tested. The bridge allowlist is
in `src-tauri/src/dev_session.rs`; add a command only after reviewing its host
side effects. Do not silently substitute mock responses for unsupported actions.
