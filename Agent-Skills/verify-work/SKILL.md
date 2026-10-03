---
name: verify-work
description: Verify a Verenu implementation before handoff, including bug fixes, UI, persistence, dictation, and native integration. Do not apply to read-only research.
---

# Verify Verenu work

Read `docs/AGENT_VERIFICATION.md` for report formats and commands.

1. Translate the request into observable acceptance criteria in a private task
   file under `test-results/`. Reproduce reported bugs before editing when
   practical. Record why reproduction was unavailable.
2. Select checks using `npm run verify:task`. Automatic requirements are a
   minimum; extend them for affected callers and platforms.
3. Add regression coverage that catches the original behavior when practical.
   Never weaken assertions, skip broken behavior, raise budgets, substitute
   mocks, or edit frozen smoke contracts to produce a passing result.
4. Inspect changed flows in your own real Rust-backed session. Check relevant
   failure, recovery, reload, and restart states. Inspect screenshots yourself.
   Capture desktop/phone views for UI changes. Use public synthetic seeds for
   shareable evidence. Keep access files and traces private.
5. For dictation, exercise relevant synthetic fixtures through the production
   pipeline. Check output, completion events, and the exact new history row.
   Require live checks when configured. Skips prove no provider behavior.
6. Native WebView tests establish WebView/IPC/window behavior only. Shortcuts,
   external insertion, clipboard/selection restoration, permissions, and mic
   behavior need affected-platform fixtures. Phone widths do not prove Android.
7. Run the completion gate against final source. Restart Rust workers after
   edits. Fix failures and inspect traces. Successful retries remain flaky.
   Rerun affected checks after further edits.

Only call work verified when the report says `verified` and acceptance outcomes
have evidence. If a required platform, credential, or fixture is unavailable,
hand off as `incomplete`, naming missing checks and completed verification.
Do not spin indefinitely on missing capabilities, call incomplete work mergeable,
or delegate routine available browser checks to the user.

Handoff includes changed behavior, source identity, checks, gaps, and evidence.
For UI changes, use `upload-file` for before/after public screenshot URLs after
inspecting them for private data.
