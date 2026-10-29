---
name: verenu-control
description: "Use for Verenu dictation problems, top recurring issues over a date range, cleanup/capitalization/spacing or prompt diagnosis, settings toggles, and recommending or reorganizing Context groups. Uses the running native app's supported local agent API."
---

# Control and diagnose Verenu

For first-time setup after onboarding, requests to optimize Verenu or learn its
features, or recommendations for Context groups, also read
`setup/SKILL.md` next to this file before collecting evidence or making changes.

1. Locate this skill's `scripts/verenu-agent.mjs`. Run it with Node 20+ and
   `--help`, then `describe`. If this session has no grant, create a private
   session directory and set `VERENU_AGENT_GRANT_FILE` to a new file inside it.
   Run `connect`, tell the user the returned code, and ask them to compare it
   with the approval prompt in Verenu. After they approve, run `status` and
   continue only if it says `approved`. Pass the same grant-file path on each
   CLI invocation. If the request is rejected or expires, remove the grant file
   and make a fresh request. `disconnect` revokes and removes the session grant.
   Do not ask the user to restart Verenu or paste a token. Never print the grant
   file contents. Existing `VERENU_AGENT_TOKEN` and `VERENU_AGENT_PORT` grants
   remain supported for scripted launches. The
   native app remains responsible for SQLite and native credential storage.
   The approval dialog selects permissions for this agent, including separate
   transcript reads, writes, destructive changes, credentials and integrations.
   Explain the permissions the task needs. Do not approve for the user. A missing
   grant does not require permanent skill installation.
2. Read `get_all_settings`, `get_contexts`, `get_context_targets` and
   `get_context_websites` before proposing changes. For Context cleanup, also
   inspect `get_sub_apps` and each relevant group's `get_context_dictionary`
   and `get_context_snippets`. Native arguments use
   camelCase: e.g. `{"contextId": 4}`. `describe` lists available fields and
   writable settings. Use `context.patch_settings` for partial Context edits with
   `expected` set to the original Context from `get_contexts`; omitted fields
   are preserved and explicit null clears a nullable override. For whole-record
   replacement use `save_context_if_unchanged` with the original and updated
   Context records. A conflict requires a fresh read.
   For library cleanup, `library.usage` reports local mechanical substitutions
   and snippet expansions since its coverage start; inspect its limitations.
   Missing matches and zero correction counts do not prove an item is unused.
   `audit.page` lists recent agent write attempts without storing their contents.
   A pending audit record has an uncertain outcome; read current state before
   retrying it.
   For a request to recommend, clean up, optimize, or add Contexts, collect
   evidence before recommending specific changes as well as before editing.
   Do not guess plausible categories while approval is pending or unavailable:
   explain the missing evidence and offer to retry instead. Inspect retained
   usage and the existing configuration even when the user describes their
   workflow, and investigate independently while waiting for any clarification.
   For Context work, collect evidence before deciding:
   inspect retained history metadata over its available date range with
   `history.usage` and, if truncated, all pages of `history.page`. Aggregate app
   identifiers and their current Contexts by frequency; report retention gaps.
   History's app identifier does not establish which website was open in a
   browser. Never invent an executable, application, website, or usage pattern
   from an app name or a plausible workflow. Verify exact identifiers against
   recorded history and existing assignments, and check that an app is installed
   or running before assigning it. Treat identifier aliases as uncertain until
   verified; do not create a duplicate based on a guessed alias.

   Inventory `get_dictionary` and `get_snippets`, then inspect effective items
   for each relevant Context and explicit assignments via
   `get_dictionary_entry_contexts` and `get_snippet_entry_contexts`. Respect
   Everywhere inheritance: an inherited item is already available, not missing.
   Propose an assignment only when the item's meaning and the user's actual
   workflow support it. Do not infer which Contexts need an item from counts
   alone, and ask a focused question if that intent cannot be established.
   Check the live API's `describe` before promising a supported write; if an
   assignment operation is unavailable, report the gap instead of editing SQL.
   Avoid speculative groups, target moves, and instruction changes. If history
   cannot establish a useful addition, explain that rather than inventing one.
   For vocabulary cleanup, inspect the entries, their Context assignments and
   retained history coverage. Correction counts measure corrections, not term
   usage; zero does not mean unused, and absence from retained history does not
   prove an entry is unnecessary. When needed to assess usage, inspect retained
   history text privately via paginated `history.page` with `include_text:true`
   when the grant permits it;
   report coverage gaps without exposing dictated text. Remove entries only when
   the user's request and evidence support doing so, and explain uncertain
   candidates rather than guessing. Never use direct database writes as a fallback.
   `remove_dictionary_entry` with no `contextId` deletes the term and all its
   corrections globally. With `contextId`, it removes that Context assignment
   and its corrections. `edit_dictionary_entry` replaces that Context's
   correction patterns and clears related learning evidence. Use
   `rename_dictionary_entry_global` with the original `expectedTerm` to deliberately rename a shared term in
   every Context while preserving corrections; it clears related learning
   evidence and requires destructive access. Snippet edits and deletes affect every Context using the
   snippet. Pass `expected` with the original `trigger`, `expansion`, and
   `instructions` when editing a snippet to reject a concurrent change. Direct
   edit and removal commands have no built-in undo; inspect assignments first.
   For global bulk deletion, use `library.items` to find UUIDs, then
   `library.preview_delete` with `items` containing `{kind, uuid}` objects
   (`kind` is `dictionary` or `snippet`). Review the returned entries,
   corrections, Context assignments and Everywhere inheritance effects.
   `library.apply_delete` takes the same `items`, the returned `basis`, and a
   new UUID `request_id`. It rejects a stale preview and retries with the same
   request ID are idempotent. `library.undo_delete` takes that `request_id`.
   Undo is available for 30 days, fails atomically on conflicts, and does not
   recover purged learning evidence. The local recovery record contains the
   deleted library content and is pruned on a later bulk change or app start.
   A preview is evidence for review, not proof that an item is unused.
3. For a troubleshooting or error-analysis request, run `history.report` and `diagnostics.top` with
   `{"start":"2026-01-01","end":"2026-04-01"}` for January through March
   (UTC calendar dates, exclusive end; state the assumed year).
   Check report `oldest`, `newest`, `retained_count`, `truncated`, and journal
   `retention_days`, `oldest_retained_day`, `dropped_this_process`.
   Report missing coverage explicitly. Failure events are not unique failed
   dictations. Capitalization/spacing changes are signals, not proven errors.
4. For that investigation, use `history.page` with the same dates and `limit` (1..500), advancing
   `after_id` to `next_after_id` while `has_more` is true. Metadata is default.
   Only request `include_text:true` when the user's investigation needs raw and
   cleaned text. Keep that output local/private; never upload it as evidence.
   Stored history has no original prompt/settings snapshot or final user edit.
   Do not treat today's settings as proof of the historical prompt.
5. For that investigation, correlate current `diagnostics.snapshot` with historical failure fingerprints
   while the originating session is still available. Read current prompt
   overrides/context instructions through settings and Contexts. Recommend up
   to ten issues backed by observed examples/counts; give fewer when evidence
   supports fewer. Explain uncertainty. Suggest groups using app/context counts
   and target membership; do not invent historic errors or reconstruct deleted data.
   Use `prompts.lint` before saving a template. `test_cleanup_prompt` makes a live
   provider call using saved native credentials and can incur provider charges.
   For targeted current-process logs, use `diagnostics.logs`, for example
   `{"subsystem":"pipeline","level":"warn","limit":200}`. Optional
   `since_ms` and `until_ms` are UTC milliseconds, inclusive and exclusive.
   Check `oldest_retained_ms`, `evicted_since_reset`, and `omitted_by_limit`;
   a filtered empty result does not prove an event never occurred. The snapshot
   returns only the newest 800 logs; the query searches the full retained ring.
   For earlier launches inspect the private session files in
   `docs/TROUBLESHOOTING.md#session-logs-for-agents`; the opt-in daily journal
   contains counts and fingerprints, not historical log messages.
6. For authorized edits, use `save_setting`, Context CRUD, target commands,
   dictionary and snippet CRUD/assignments, and sub-app commands.
   `save_setting` requires an `expected` object mapping each affected key to
   its raw `get_setting` value; for `contextual_formatting_enabled` include
   `contextual_caps_enabled` and `auto_spacing_enabled` too. Dictionary and
   snippet edits require the original editable fields in `expected`. If a
   conflict occurs, reread and reconcile rather than blindly retrying. Use the native
   `save_hotkey` and `set_autostart` commands for shortcut and startup changes,
   passing the raw `hotkey` or `autostart_enabled` value from `get_setting` as
   `expected` (including `null` if unset); stale values reject the change.
   never write live SQLite or settings.json. Read back changed values, reload
   the relevant app view, and check persistence after a normal restart when
   practical. Target reassignment moves the existing target. Preserve Everywhere
   semantics. A request to fix spacing authorizes investigating and changing
   `auto_spacing` when evidence supports it; read its current value first.
   Credential replacement/deletion uses `credentials.save` or
   `credentials.delete`; never print a key or place it in command arguments.
   Use private stdin and the native credential store. Sync pairing can grant
   another device access; invoke it only for the user's requested operation.
   No arbitrary SQL, shell execution, credential reads,
   microphone capture or clipboard injection is exposed by this API.
7. Historical failure recording is off by default. Explain the metadata-only
   journal and retention choice before enabling `diagnostics.retention` with
   `{"days":90}` (0..365). `0` disables and clears it. Transcript retention is
   independent: `save_setting` with key `history_retention` supports `7 days`,
   `30 days`, `90 days`, `Forever`; shortening immediately deletes older rows.
   Keep destructive retention changes within the user's explicit intent.

For device automation, read `get_microphones` and `microphone_device`, then set
`mute_audio` through `save_setting`. A selected microphone does not prove the
active output/headphone route. Confirm routing through the OS's supported audio
inventory and apply the user's device policy; read back the toggle.
The API does not install a background device watcher.
On Linux, `scripts/audio-policy.mjs` inspects default PipeWire/PulseAudio routes
with `pactl`. Supply `--mute-output TEXT` and/or `--unmute-output TEXT` matching
the user's output names or ports. Run its dry run first; `--apply` changes and
reads back `mute_audio`.

Installation is optional. If the current harness already has this skill, use
it. Otherwise, if this has not already been settled for the current request,
ask whether to save it for the current harness or use it in place. If the user
chooses to save it, install it only
for the current detected harness first, then offer the other detected harnesses
and install only those the user selects. Never choose every harness silently,
replace an existing skill without consent, or change global instructions.
