---
name: verenu-control
description: "Use for Verenu dictation problems, top recurring issues over a date range, cleanup/capitalization/spacing or prompt diagnosis, settings toggles, and recommending or reorganizing Context groups. Uses the running native app's supported local agent API."
---

# Control and diagnose Verenu

1. Locate this skill's `scripts/verenu-agent.mjs`. Run it with Node 20+ and
   `--help`, then `describe`. Use the user's existing local grant through
   `VERENU_AGENT_TOKEN` and `VERENU_AGENT_PORT`. If unavailable, explain how to
   start Verenu with a grant; never request or print the token in chat. The
   native app remains responsible for SQLite and native credential storage.
   The grant enables private reads and writes: do not enable it without the
   user's agreement. A missing grant does not require permanent skill installation.
2. Read `get_all_settings`, `get_contexts`, `get_context_targets` and
   `get_context_websites` before proposing changes. Native arguments use
   camelCase: e.g. `{"contextId": 4}`. `describe` lists available fields and
   writable settings. Omitted optional fields in `update_context_settings`
   become null, so preserve existing fields when editing one property.
3. For a period, run `history.report` and `diagnostics.top` with
   `{"start":"2026-01-01","end":"2026-04-01"}` for January through March
   (UTC calendar dates, exclusive end; state the assumed year).
   Check report `oldest`, `newest`, `retained_count`, `truncated`, and journal
   `retention_days`, `oldest_retained_day`, `dropped_this_process`.
   Report missing coverage explicitly. Failure events are not unique failed
   dictations. Capitalization/spacing changes are signals, not proven errors.
4. Use `history.page` with the same dates and `limit` (1..500), advancing
   `after_id` to `next_after_id` while `has_more` is true. Metadata is default.
   Only request `include_text:true` when the user's investigation needs raw and
   cleaned text. Keep that output local/private; never upload it as evidence.
   Stored history has no original prompt/settings snapshot or final user edit.
   Do not treat today's settings as proof of the historical prompt.
5. Correlate current `diagnostics.snapshot` with historical failure fingerprints
   while the originating session is still available. Read current prompt
   overrides/context instructions through settings and Contexts. Recommend up
   to ten issues backed by observed examples/counts; give fewer when evidence
   supports fewer. Explain uncertainty. Suggest groups using app/context counts
   and target membership; do not invent historic errors or reconstruct deleted data.
   Use `prompts.lint` before saving a template. `test_cleanup_prompt` makes a live
   provider call using saved native credentials and can incur provider charges.
6. For authorized edits, use `save_setting`, Context CRUD and target commands;
   never write live SQLite or settings.json. Read back changed values, reload
   the relevant app view, and check persistence after a normal restart when
   practical. Target reassignment moves the existing target. Preserve Everywhere
   semantics. A request to fix spacing authorizes investigating and changing
   `auto_spacing` when evidence supports it; read its current value first.
   No arbitrary SQL, shell execution, credential reads/writes,
   microphone capture or clipboard injection is exposed by this API.
7. Historical failure recording is off by default. Explain the metadata-only
   journal and retention choice before enabling `diagnostics.retention` with
   `{"days":90}` (0..365). `0` disables and clears it. Transcript retention is
   independent: `save_setting` with key `history_retention` supports `7 days`,
   `30 days`, `90 days`, `Forever`; shortening immediately deletes older rows.
   Keep destructive retention changes within the user's explicit intent.

For device automation, read `get_microphones` and `microphone_device`, then set
`mute_audio` through `save_setting`. A selected microphone does not prove the
active output/headphone route. Use the OS's supported audio inventory to confirm
Pebble V3 output versus Yeti Nano/headphones before deciding, and read back the
toggle. The API provides primitives; it does not install a background device watcher.
On Linux, `scripts/audio-policy.mjs` inspects default PipeWire/PulseAudio routes
with `pactl`. Run its dry run first; `--apply` changes and reads back `mute_audio`.

Installation is optional: run `harnesses` or `install-skill`. Interactive setup
asks whether installation is permanent and which detected harnesses to use.
Never choose every harness silently or change generated global instructions.
