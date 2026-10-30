# Verenu Data And Privacy

This document explains what Verenu keeps on device, what it sends off device, and what it does not do.

## Core Principles

- Verenu's own server (`api.verenu.com`) serves only public app metadata — release info, download links, and provider status. It never receives your dictated audio, transcripts, API keys, or history.
- Desktop and Android builds may include optional pseudonymous product
  analytics when the build is configured with the public PostHog ingestion
  token; this is limited to explicit
  aggregate events and is described below.
- Your dictated audio and text either stay on your machine or go directly to the AI providers you configure — never through a Verenu server.
- Safety defaults beat convenience. On Windows, updates download or open the published installer instead of auto-executing downloaded bytes.

## What Stays On Device

### API keys

- Windows: stored in Windows Credential Manager
- macOS: stored in Keychain
- Linux: stored through Freedesktop Secret Service (GNOME Keyring/KWallet)
- Custom provider keys use separate credential-store entries identified by a stable provider UUID. Renaming a provider keeps its key. Removing a provider deletes its key.
- Legacy plaintext storage is migrated away from older formats where possible

### App data

Stored locally in app storage and SQLite:

- Settings
- Provider and model preferences
- Custom provider definitions, including base URLs, task-specific model IDs, and non-secret request options. Definitions are included in settings backups; their API keys are not. Custom definitions are device-local and do not participate in LAN settings sync.
- Context groups, their app/website targets, per-context tone/cleanup overrides, vocabulary, and snippets
- Transcription history
- When available, the hostname of the browser page active at the start of a dictation is stored with that history entry. Full URLs, paths and query strings are not stored as website metadata. Detection can be unavailable; older entries are not inferred. This hostname follows history retention, backup and device sync.
- An optional metadata-only agent failure journal. It is off by default and
  stores daily UTC counts, fixed subsystem/category labels, and diagnostic
  fingerprints for up to 365 calendar days, including today. It has no history
  backfill and stores no error messages, prompts, transcripts, paths, model
  names, or credentials. Disabling it clears its rows logically; it does not
  promise secure erasure of backups or disk blocks. See [Agent control](AGENT_CONTROL.md).
- Auto-learn events and candidate data
- Update-dismiss state
- A short-lived dictation failover spool (`dictation-failover/` under the app data directory): in-progress audio so a crash or reboot can offer Continue. Deleted after history is saved, on dismiss, or after 24 hours. Not included in backups.

### Cleanup cache

Privacy settings can disable persistent cleanup caching and clear its stored
results. Caching is enabled by default and stores cleaned text locally in SQLite.
Keys are versioned SHA-256 hashes of exact input and cleanup configuration;
readable raw transcripts are not stored as keys. Hashes do not encrypt the
cleaned text or make predictable inputs anonymous.

Paired devices exchange cached cleaned text over authenticated, hop-by-hop TLS
connections. Disabling caching clears this device's cache and stops it storing
or forwarding cached results. It does not erase copies already sent to peers.
Rejection records, including cache-key hashes, still travel through disabled
devices to help remove rejected results elsewhere. The setting stays local to
each device.

Results expire after two days without reuse or seven days from creation,
whichever comes first, including snippet results. The cache is limited to 2,000
entries and 16 MiB of key/output text. The displayed size measures that text,
not the SQLite file. Clearing removes cache rows; it does not promise secure
erasure of database pages or delete transcription history.

Reuse and provider-duration counters stay in process memory and reset on app
restart. They contain no dictated text and are not sent as analytics.

### Logs

- Recent logs stay local unless you explicitly export them.
- Redacted session logs are written automatically to `logs/` under the app
  data directory. Each launch starts fresh files. Files are retained for up to
  30 days within a 256 MiB budget; low disk space pauses persistence. See
  [session log locations and limits](TROUBLESHOOTING.md#session-logs-for-agents).
- Exported logs are created only when you trigger that action.
- Unlocking Developer mode does not enable verbose logging by itself.
- Verbose logging must be enabled explicitly from the Developer panel.

Current logging paths are intended to use redacted metadata rather than raw private content. That means counts, ids, model names, app identifiers, filenames, and redacted path labels are preferred over dictated text, prompt bodies, raw dictionary terms, raw snippet expansions, or full local paths.

Even with that hardening, log exports can still contain sensitive operational detail. Do not share them casually.

### Optional local agent control

Desktop builds offer a local agent API on `127.0.0.1`. An agent must request
access and the user must approve the matching code in Verenu. Approval grants
access for one hour or until the app closes; Settings > Privacy can revoke it
sooner. Verenu keeps approved grant hashes in memory, and the agent keeps its
grant in a private session file. Scripted launches can instead provide an
explicit bearer grant and port. A local process holding a grant can read app settings (including
prompt and custom-provider configuration), Contexts, the current redacted
diagnostics snapshot, and retained history, and can change allowlisted settings,
Contexts, vocabulary, snippets, sub-apps and sync configuration. It can replace
or delete API keys in the native credential store but cannot read them back.
The grant is not a boundary against a malicious process running as the same OS
user, which may be able to inspect process environments or access app data.
History text is returned only when the caller explicitly requests it. The API
has no credential-read, arbitrary SQL,
shell, audio-capture, or text-injection operation. Requests stay on the device;
the prompt-test operation can make the same provider request as the app and
send its test prompt to the configured provider. See [Agent control](AGENT_CONTROL.md)
for installation, access, bounds, and retention details.

### Backups

Manual export and import stay local unless you choose to move the file elsewhere.

Current backup export includes:

- Settings
- Context groups, app/website targets, context assignments, context-specific
  vocabulary and AutoLearn correction mappings, and snippets
- Retained transcription history, including raw and cleaned text, timestamps,
  Contexts, and provider usage records. The backup file is sensitive.
- Lifetime, daily, Context, hourly and provider Insights, including summaries
  retained after history is pruned. Import deduplicates retained records by UUID.
  Historical summaries do not identify individual dictations; overlapping
  summaries are reconciled conservatively and reported when uncertain, so some
  totals can be incomplete. Imported history remains subject to retention.
  Per-device counter IDs are included to reconcile totals with LAN sync without
  counting the same device twice. Pairing credentials and certificates are not
  transferred. If a source device has no counter identity, its aggregate totals
  cannot safely be merged and the import report flags the limitation. Older
  backup versions contain only an unidentifiable total; their settings and
  library still import, but that total cannot safely be restored.

After history retention removes transcript text, Verenu keeps a numeric
receipt for each dictation and provider call. These receipts contain an event
identifier, calendar bucket, Context, and usage measurements so that backups
and LAN sync can recognize activity already counted on another device. They do
not contain transcript text, prompts, audio, or application names. Older sync
peers do not understand receipts. A fresh sync snapshot is requested when a
peer upgrades, so it can receive activity it missed on the older version.
History with ambiguous older statistics syncs separately, so its text remains
available without counting its uncertain contribution twice. Incoming history
outside the receiving device's retention period is kept as a text-free receipt.

Short-lived AutoLearn candidate/pending evidence is intentionally local-only
and is not exported or synchronized. Pending correction evidence expires after
30 days. Persistent promoted mappings retain their originating Context during
backup restore and LAN sync.

Backups exclude API keys, device pairing credentials, local microphone
and secondary shortcut selections, downloaded models, temporary caches, pending learning
evidence, the device's opt-in diagnostic failure journal, and the in-progress
audio spool. They also exclude sync cursors and logs, unresolved sync
dependencies, cached provider pricing, and seeded-default bookkeeping. These
are credentials, device-local diagnostics or sync state, replaceable cache
entries, or transient data. App targets are matched to installed apps on the
destination; weak, ambiguous, or conflicting matches are left out and
reported.

Import and restore paths validate supported setting values and reject oversized prompt overrides, snippet bodies, and unsupported context-target values instead of silently accepting junk. Legacy app-mapping exports remain supported for migration.

## What Leaves Your Device

### Audio

When you finish a dictation, Verenu either transcribes audio locally or sends recorded audio to the transcription provider you selected.

That can be:

- Local Parakeet V3
- Groq
- OpenAI
- Google
- AssemblyAI
- OpenRouter, which forwards the audio to the speech model you choose there
- xAI

If transcription is local, audio stays on the device after the model download.

### Text sent to cleanup models

After transcription, Verenu can send text to a cleanup model so it can:

- remove filler words
- fix punctuation
- apply formatting rules
- apply context instructions, vocabulary, and snippet instructions
- apply tone or cleanup intensity

That means raw transcription text leaves your device when cleanup is enabled, including when transcription itself ran locally.

With Dual model transcription enabled, the same audio may be sent to two or more providers from the configured transcription chain until two candidates succeed. Both successful candidates are then sent to the selected cleanup provider. Failed candidates do not fail a successful transcription.

### Optional context

Depending on your settings and the feature being used, Verenu may also send:

- context tone and cleanup settings
- context instructions, vocabulary, and snippet instructions
- selected model metadata

### Update checks and downloads

Verenu checks GitHub release metadata for updates.

That request does not include your dictated text, history, snippets, or API keys.

On Windows and macOS, installing an update opens the published GitHub asset so the platform installer flow can take over. Verenu does not auto-run a downloaded Windows executable from a fixed temp path.

### T3 Code skills

Settings > Integrations can pair with a T3 Code environment using its existing
pairing link. The bearer credential stays in the native credential store. Verenu
stores the connection address, environment identity, provider/workspace catalogs,
skill names and descriptions, and refresh timestamps locally. Connection data and
credentials are excluded from backup exports and LAN sync; the Context opt-out
preference is included.

Verenu refreshes metadata daily while running, with a manual **Pull now** action.
The last imported catalog remains available offline until a successful pull
replaces it or you disconnect. Credential expiry and failed pulls require
reconnecting to refresh, but do not erase saved skills. A failed pull from a
changed environment retains the previously paired environment's catalog.
Public addresses require HTTPS; localhost and private LAN addresses can use HTTP.
Tailscale is optional.

Verenu combines all imported provider/workspace catalogs into one shared skill
list, deduplicated by name. No workspace selection is required. Only dictation
captured in the T3 desktop app receives this list.
With cleanup enabled, only skill identifiers are sent to the configured
cleanup provider as matching evidence. Descriptions remain local for browsing;
skill bodies are never downloaded. Clear
requests to invoke a known skill can become `$skill-name`. The cleanup model
chooses the closest confidently matching exact name from the full imported list
and abstains when the request is unclear; ordinary speech and references that
are quoted, negated, or discussed should not add a skill token. The output guard
rejects newly invented identifiers while preserving existing dollar tokens and
spoken environment-variable references. Skills are treated as shared across T3
conversations; pairing does not identify the active conversation.
Browser-hosted T3 windows are not detected as desktop T3 targets.

Skill conversion requires a working cleanup provider. If all cleanup providers
fail, Verenu preserves the transcript without adding skill mentions. Default
logs record whether a T3 catalog was captured and attached, without skill names
or dictated text.

### GitHub contribution insights

Insights can optionally compare dictated words with the contribution calendar
shown on a public GitHub profile. This is off until you enter a GitHub username.
Verenu sends that username and a date range to GitHub's public profile page
without credentials. The calendar can include anonymized private contribution
counts only when the profile owner has enabled **Show private contributions**
on GitHub. Those counts do not identify private repositories or reveal their
names. No dictated text or local Context information is sent. Verenu stores the
username and up to 90 days of daily counts locally; commit messages and
repository names are neither stored nor returned to the UI.

If the profile calendar cannot be read or does not cover the requested dates,
Insights falls back to GitHub's public commit-search API. That source can miss
activity outside GitHub's public index and is limited to 1,000 results. Partial
search results preserve returned counts as incomplete values and leave days
without indexed results unknown. The UI labels this source **Public commits**.
Counts refresh automatically every 15 minutes while the app is running, and
cached counts remain available when GitHub cannot be reached with their source
label intact. Disconnecting removes the saved username and cached counts. These
settings are excluded from device sync and settings exports.

Settings > Integrations can suggest the active GitHub CLI username, or an
explicit global `github.user` Git setting. The CLI authenticates its own request
to GitHub; Verenu receives only the username and never reads its credentials.
Suggestions do not connect an account until you choose Connect GitHub.

### Context website checks

When you attach a website to a context group, Verenu resolves the domain over DNS before accepting it, so a typo can't create a website target that will never match anything. This is a plain DNS lookup, not an HTTP request — it does not fetch the site, send cookies, or reveal your IP to the site owner beyond what any DNS resolution already does. Nothing about your dictation, history, or other settings is included; only the domain you typed leaves your device, to your configured DNS resolver.

### Website favicons

To show a real site icon next to a website context target (in the sidebar and on the context page), Verenu asks Google's public favicon service for that site's icon. Only the bare hostname you attached (for example `mail.google.com`) is sent — never a full URL, path, dictation, history, or API key — and the site itself is never contacted.

Each hostname is requested at most once: the result is cached on disk under the app data directory, including the "this site has no icon" outcome, so the icon is not re-fetched when the sidebar rerenders, you switch pages, or you restart the app. If the lookup fails, the row falls back to a generic globe glyph.

### Connectivity check

Verenu first reads the operating system's network state. Windows uses the Network List Manager and macOS checks route reachability. These checks do not send traffic.

If a real provider or Verenu service request fails, Verenu can run a short active probe to distinguish a local connection problem from a provider outage. When service checks are enabled, it checks Verenu's health endpoint first, then uses `www.google.com` as an independent second check. This probe is limited to the failure path and carries no dictated text, history, snippets, or API keys.

### Provider status checks

Verenu periodically asks `api.verenu.com` for the operating status of the transcription and cleanup providers you have selected:

- Settings → Privacy includes **Allow Verenu service checks**, enabled by default. Turn it off to stop these background requests. Verenu clears the in-memory status and health results when you disable the setting.
- Every 5 minutes, it fetches provider status and shows an in-app banner only if the status API flags a real problem for a provider you have actually selected. A provider reporting `operational` or `unknown` does not trigger a banner, and providers you have not selected never surface regardless of their status.
- Every 20 minutes, it does a plain up/down health check of `api.verenu.com` itself. This currently has no UI; the result is kept in memory for future features.
- If a transcription or cleanup call fails in a way that looks provider-side (a quota error, or a retryable timeout/429/5xx), Verenu immediately re-checks provider status instead of waiting for the next scheduled poll.

These are plain GET requests with no request body. They never include your dictated audio, transcripts, history, dictionary, snippets, prompts, or API keys — Verenu's server only ever sends back public status data in response, it does not receive anything from you beyond the bare HTTP request.

### Optional desktop and Android product analytics

Product analytics requires a configured PostHog ingestion token and host.
Desktop builds use the shared Rust transport; Android uses its native SDK
boundary. It is enabled by default in configured builds and can be disabled
during onboarding or in Settings -> Privacy.

Analytics sends approved usage events, safe settings summaries, and sanitized
failure reports. It uses a persisted random installation ID, a fresh process
session ID, and per-dictation run IDs. The installation ID allows cross-launch
usage measurement. These IDs are not derived from accounts, hardware, pairing
identity, or user content. Opt-out deletes the installation identity and stops
new events; re-enabling creates a new identity. Previously queued Android
events may remain in its offline queue.

Dictated audio and text, prompts, clipboard contents, history, vocabulary,
snippets, app/window contents, URLs, credentials, microphone names, and raw
error messages are excluded. The [analytics contract](ANALYTICS.md) documents
the exact event allowlists, sanitized Error Tracking fields, transport controls,
identity storage, and opt-out behavior. Analytics is pseudonymous, not
mathematically anonymous.
See [the event contract](ANALYTICS.md) for the complete allowlist.

## History Loading

The Home view loads recent transcription history in pages of 100 items by default and can request older pages on demand.

This changes UI loading behavior, not storage location. The full history database still lives on your machine unless you export or delete it.

## What Verenu Does Not Send

`api.verenu.com` is in the product path today (release/update metadata and provider status), but even so, Verenu does not send any of this to that server or any other Verenu-owned server:

- transcription history
- context vocabulary and snippets by default
- local settings backups by default
- analytics events to Verenu-owned servers, optional product events go to the separately configured PostHog host
- user profiles
- payment data

That said, once data is sent to a third-party AI provider, that provider's retention and privacy rules apply. Verenu cannot override that.

## Data Map By Feature

| Feature | Stays local | Leaves device |
| --- | --- | --- |
| Hold-to-record audio capture | audio before release, plus a local crash-recovery spool until the take is saved, dismissed, or 24 hours old | nothing until transcription starts |
| Local transcription + Cleanup Off | audio, transcript, settings, and history | nothing after the model download |
| Local transcription + cloud cleanup | audio, local model, local capture state | transcript text and cleanup context to selected cleanup provider |
| Cloud transcription | local capture state | audio to selected transcription provider |
| Cleanup | local settings and local cache | raw transcription text and cleanup context to selected cleanup provider |
| Context vocabulary and snippets | SQLite | nothing by default |
| Context website check | current app state stays local | the typed domain, via a plain DNS lookup, when you attach a website to a context group |
| Auto-learn | local monitoring data and promoted entries | nothing by default |
| Update check | current app state stays local | GitHub release metadata request |
| Connectivity check | current app state stays local | native OS network-state check; a short active probe only after a real request fails |
| Provider status check | current app state stays local | optional periodic GET to `api.verenu.com/v1/provider-status` (every 5 min, plus an immediate recheck after a provider-side pipeline failure) |
| API health check | current app state stays local | on-demand GET to `api.verenu.com/v1/health`; no automatic polling |
| Export data | backup file on local disk | nothing unless you share the file yourself |
| Logs export | log file on local disk | nothing unless you share the file yourself |
| Optional agent control | settings, Contexts, and explicitly requested history stay in the local app and local caller; the failure journal is opt-in and metadata-only | local loopback only, except when its prompt-test operation calls the configured AI provider |
| Optional desktop/Android product analytics | random installation identity, first-seen version, milestone state, and local queues | approved events with random installation/session/run IDs and sanitized failure reports; see [contract](ANALYTICS.md) |

## If You Change Privacy Behavior

If you contribute code that changes any of the following, update this file and the README:

- what data leaves the device
- which provider receives what
- what gets stored locally
- backup or export contents
- logging behavior
- new network calls
- updater download or installer behavior
- local transcription model behavior or privacy claims

If you cannot explain the data flow in plain English, the feature is not documented well enough yet.

## Related Docs

<p align="center">
  <a href="PRIVACY_SUMMARY.md"><img alt="Privacy Summary" src="https://img.shields.io/badge/Privacy-Summary-c44632"></a>
  <a href="API_KEYS.md"><img alt="API Keys" src="https://img.shields.io/badge/API-Keys-5b554a"></a>
  <a href="TROUBLESHOOTING.md"><img alt="Troubleshooting" src="https://img.shields.io/badge/Help-Troubleshooting-7e7266"></a>
  <a href="README.md"><img alt="Docs Index" src="https://img.shields.io/badge/Docs-Index-2b2422"></a>
</p>
