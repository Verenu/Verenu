---
name: verenu-install
description: Use when asked to set up Verenu after onboarding, improve or explore its features, or manage or troubleshoot its dictation, Context groups, or application settings. Finds Verenu's packaged control and setup skills. Do not use for unrelated Verenu source-code work.
---

# Find Verenu's control skill

Verenu packages a `verenu-control` skill with its native app. Find its
`SKILL.md` and `scripts/verenu-agent.mjs` in the installed application's
`agent-skill` resources (or `Agent-Skills/verenu-control` in a source checkout).
On Linux, resolve the `verenu` launcher and inspect its installation directory;
AppImage resources are inside the mounted or extracted image. On macOS, inspect
`Verenu.app/Contents/Resources/agent-skill`; on Windows, inspect the installed
Verenu resources. Do not assume the current directory is a source checkout.

Read the packaged control skill before acting. Tell the user you found it and
ask whether to save it for the current harness or use it from its present
location. If they decline, use it in place for this request. Do not install the
control skill without consent, and do not change global agent instructions.
For setup, optimization, feature recommendations, or Context suggestions,
also read `setup/SKILL.md` inside the packaged control skill.

If they choose to save it, identify the current harness and run the packaged
script's `harnesses` command. Install only that harness with `install-skill
--permanent --harness <id>`. If it is not detected, explain the limitation and
use the packaged skill in place. Then tell the user which other supported
harnesses were detected and ask which, if any, should also receive it. Install
only the harnesses explicitly selected. Existing skills are not replaced
without separate consent. A noninteractive agent should ask in conversation,
not rely on the script's terminal prompts.

Follow the control skill's grant and verification instructions for the user's
actual request. Saving a skill does not enable agent access to Verenu.
