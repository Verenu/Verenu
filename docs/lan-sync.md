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
  `src-tauri/src/sync/engine.rs`, including language, cleanup preferences,
  smart spacing and capitalization, and the cleanup prompt.

Provider and model selections, fallback models, and dual transcription stay
local because each device has its own credentials and available local models.
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

Committed database edits and local counters are checked every 750 ms while
paired. Either device can initiate a session after an edit, including changes
made during a running session. Quiet pairs use one deterministic initiator
every 30 seconds. Failed sessions retry with backoff and different initial
delays on each side to recover from simultaneous edits. Manual Sync now
waits for the transfer result and reports offline devices or transfer errors.

## Persistent Tailscale connections

An already paired device can have a saved Tailscale IPv4 address and Verenu
sync port. This route is device-local and survives restarts. It is tried before
LAN discovery and works when mDNS cannot cross networks. The existing TLS
certificate pin and pairing trust still authenticate the peer. This uses the
sync listener, not the browser development backend or HTTP.

### Setup

1. Install Tailscale on each device and connect them to the same tailnet.
   Enable Sync in Verenu, then open **Settings > Sync > Connect through Tailscale**.
2. Check this device's Tailscale IPv4 address against the Tailscale app. If
   Verenu cannot detect it, paste it into the address field. The sync port is
   shown on this page; no Tailscale Serve or HTTPS configuration is required.
3. Copy **connection details** from device A. On device B, paste them into
   **Other device's connection details** and select **Pair connection**.
4. Enter the short pairing code shown on B into the approval prompt on A.
   This authenticates the devices and saves B's route to A.
5. Copy B's connection details back to A. Under B in **Paired devices**, select
   **Connection**, paste the details, and **Save connection**. Now either side
   can reconnect without nearby discovery. Existing LAN pairs can use this
   Connection editor on both sides without pairing again.
6. Wait for **Up to date**, then create a Context on either device and check
   that it appears on the other without selecting Sync now.

Only `100.64.0.0/10` IPv4 addresses with a nonzero port are accepted. Connection
details include a device ID and listener address, never a secret or API key.
They do not grant access without the code exchange. The editor rejects details
for a different paired device. An empty connection restores LAN-only routing.
MagicDNS names and HTTPS URLs are not accepted by this raw TLS transport.

Both devices must be running Verenu and connected to Tailscale. Firewalls and
tailnet access rules must allow the shown TCP sync port from the other device.
If the address or listener port changes, copy fresh details and update the
connection on its peers. Do not expose the browser development backend.

### Three or more devices

Pair each new device with an existing device that will stay online. For example,
pair a phone and two laptops with the same desktop, saving both directions of
each connection. Contexts, history, shared settings, deletion records, and
per-origin counters relay through paired devices. Relaying data does not pair
or grant direct access to another device automatically.

If that desktop is offline, its peers cannot relay through it. Add pairings
between the other devices to keep them connected independently. A full mesh
has three links for three devices and six links for four. Offline edits sync
when a path of online paired devices becomes available again.

On Android, keep Verenu open while syncing. Nearby discovery holds a Wi-Fi
multicast lock only while the activity is visible. Saved Tailscale connections
use unicast. Android may suspend or kill the process in the background; this
feature does not promise continuous background sync or wake a sleeping app.

Connection errors identify the attempted address and TCP port and explain
that a firewall or network rule may be blocking it. A timeout cannot prove
which rule blocked traffic, so Verenu does not claim a confirmed firewall cause.
Alternate addresses are tried only when the TCP connection fails. Once a peer
responds, a busy session, identity mismatch, or transfer failure ends that
attempt; trying its other addresses would delay recovery or hide the failure.

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
Another three-device chain checks bidirectional edits and relayed Context
deletions without transitive pairing trust. A four-device mesh checks concurrent
independent edits, offline edits, database reopen, stale gossip, deletion
preservation, history and counter deduplication, and stable logs on repeat sync.
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
