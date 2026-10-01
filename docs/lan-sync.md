# LAN device sync

Verenu pairs devices on the same local network without an account or cloud
server. Discovery uses mDNS. Data travels over TLS, and paired devices verify
the certificate fingerprint recorded during the code-based SPAKE2 exchange.
The approval wait uses the full three-minute pairing deadline. The responder
only sends pairing confirmation after peer trust and initial settings stamps
are committed together.

## What syncs

- Contexts, their styles and instructions, websites, app targets, vocabulary
  and snippet assignments.
- Canonical vocabulary, Context-owned correction mappings, and snippets.
- Sub-app rules and their Context assignments.
- Dictation history and associated API usage records.
- Per-device lifetime word and correction counters, merged without counting
  imported dictations twice.
- The settings listed in `SYNCABLE_SETTINGS` in
  `src-tauri/src/sync/engine.rs`, including provider and model selection,
  language, cleanup preferences, fallback models, and the cleanup prompt.

Credentials stay in the OS credential store. Microphone selection, hotkeys,
appearance, permissions, downloaded models, and local history retention do
not sync. A cloud model selection does not transfer its API key. App targets
are matched to apps installed on the receiving device; unmatched targets
retain an unresolved marker for later repair.

## Transfer and merge behavior

The SQLite change log tracks stable row UUIDs and last-writer-wins stamps.
Both directions have separate persistent cursors. New or stale peers receive
paginated snapshots, followed by incremental changes on later sessions.
Snapshots acknowledge their starting log position so edits made during
enumeration remain eligible for subsequent deltas.

A batch commits atomically before acknowledgement. Failed batches roll back.
The receiver only advances its cursor after the final batch. Interrupted
transfers replay safely. A Context whose vocabulary or snippets have not
arrived yet stays eligible for replay, preserving its assignments across
batch boundaries. Retained deletion records prevent an old snapshot from
restoring deleted content. History retention remains device-local.

Automatic sessions use one deterministic initiator per device pair to avoid
competing connections. Failed sessions retry with backoff. Manual Sync now
waits for the transfer result and reports offline devices or transfer errors.

## Automated regression tests

Run the focused suite with:

```bash
npm run test:sync
```

The suite is also part of ordinary Rust tests and the existing CI Rust gate.
The two-device fixture in `src-tauri/src/sync/device_tests.rs` creates separate
temporary SQLite databases and disposable certificates. It opens actual TCP
connections on loopback using ephemeral ports, runs the production TLS and
SPAKE2 code, records peer trust, and runs the production session engine.
It never opens the user's database or credential store, broadcasts discovery,
records audio, injects text, or calls a provider. Connections and sessions
have test timeouts, and temporary device directories are removed afterward.

The fixture covers correct and incorrect pairing codes, more than one batch,
bidirectional offline edits, persistent cursors after database reopen,
settings, scoped vocabulary and corrections, snippets, websites, history,
API usage, lifetime totals, interrupted snapshots, and repeated sessions
without duplicate rows or log amplification. It compares stored content and
relationships independently of each device's local SQLite IDs.

A three-device case verifies relayed content and stale counter gossip.
Lifetime counters merge by maximum per origin so an older device's copy
cannot reduce totals. Settings-save failures and unresolved dependencies fail
the session instead of reporting completion; subsequent sessions retry.
Pairing persistence failures do not confirm trust on the initiating device.

Additional engine tests cover snapshot edits during enumeration, batch
rollback, missing dependencies, app-target removals, stale snapshots,
natural-key conflicts, deletion records, protocol identity checks, and
premature completion messages. The incremental test preserves existing peer
rows rather than re-pairing before each session.

## Physical two-device check

Use the same app build on both devices and a private network that permits
local device traffic. Enable sync, open Settings > Sync, pair using the code,
and wait for both devices to report completion.

1. Create a Context with vocabulary, a correction, a snippet, and a website
   on device A. Confirm both the content and assignments on device B.
2. Edit the Context on B. Confirm the edit on A. Remove a website or app
   target and confirm the removal on the other device.
3. Make a synthetic dictation on each device. Check history and lifetime
   totals, then repeat Sync now and confirm totals do not grow again.
4. Take B offline, edit different items on both devices, restart B, and
   reconnect. Confirm both edits arrive without duplicates.
5. Interrupt a larger transfer, reconnect, and confirm it finishes. Remove
   and re-pair a device with older data to check retained deletions.

The single-machine fixture verifies transport and persistence. Physical
devices still need to verify mDNS across the actual network, firewall rules,
OS credential-store access, app matching across operating systems, sleep
and wake, and the native UI. Settings persistence is represented by a test
host in the fixture. Database reopen is tested; restarting two complete
native application processes is not simulated.

The merge model uses wall-clock timestamps with deterministic tie breaking.
Large clock differences and simultaneous conflicting edits remain areas for
the next reliability pass. Last-writer-wins does not preserve both versions
of a conflicting edit.
