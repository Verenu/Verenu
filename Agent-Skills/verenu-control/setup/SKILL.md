---
name: verenu-setup
description: Help a user set up or get more from Verenu after onboarding, including evidence-based Context, vocabulary, snippet and feature recommendations.
---

# Help someone get started with Verenu

Use this for requests to set up, optimize, explore unused features, or recommend
Context groups. Read the adjacent control skill first and use its supported API.
Make recommendations first; a request for recommendations does not authorize
changing settings, creating groups, or installing skills.

## Start with privacy and the person's goals

Briefly explain what you can inspect and why, then request Verenu access and
gather the evidence below before suggesting specific Contexts, targets, terms,
snippets, or feature changes. Ask one focused question if the person's main
dictation activities or desired outcome are unclear, but do not wait for the
answer before collecting evidence that does not depend on it. Work with Verenu's
existing settings, Contexts and retained metadata first. Do not read transcript
text merely to make generic recommendations. If a specific finding requires
text, explain why and ask before reading the minimum needed; do not quote or
upload private text, history, screenshots, or machine inventories as evidence.

Browser history, when useful, is a separate optional local signal. Explain the
scope and ask before reading it; do not open browser databases or synced account
history without permission. Explain that information shown to the agent may be
processed by its model provider, even when the history is read locally. Aggregate
locally and expose only the domains and counts needed for the task, subject to
the user's consent. Never send browsing history to Verenu. Avoid private browsing,
URLs, page titles, search queries, and account details. Let the user provide a
few sites instead. If
permission, safe access, or adequate evidence is missing, proceed with less data
and say what is unknown.

## Gather evidence before recommending

Treat this as a required investigation, not an optional source of supporting
details. Check the existing setup and available usage first, including when the
user's prompt sounds casual or they already described their workflow. Do not
propose plausible categories from general assumptions while access is pending.
If access is declined, expires, or fails, say that personalized recommendations
are not yet grounded in their Verenu data; offer to retry or ask for only the
missing facts. Do not present hypothetical groups as findings about the user.

1. Inventory existing Contexts, target assignments, sub-app rules, settings,
   vocabulary and snippets, including Everywhere inheritance. Preserve existing
   intent and note conflicts before suggesting changes.
2. Use `history.usage` on an appropriate retained period, report its coverage,
   and distinguish a small sample from a real pattern. If truncated, paginate
   metadata with `history.page`. Recorded browser app usage does not establish
   which sites were dictated into. Visits alone do not establish dictation.
3. If useful, inventory locally installed or running applications by their
   platform's supported mechanisms. Installed does not mean used for dictation.
   Match exact recorded identifiers to real apps and existing assignments;
   never guess aliases, duplicate an existing target, or invent an application.
4. Compare effective vocabulary and snippets across Contexts. An Everywhere
   item already applies everywhere; absence from an explicit assignment list
   does not make it missing. Suggest a new or additional assignment only when
   the user's language and workflow support it. Do not invent terms or snippets.
5. Compare enabled features with observed workflow and ask a focused question
   where a useful recommendation depends on intent. Do not describe disabled
   features as underused solely because they are disabled.

Recommend only a few changes with a concrete benefit over existing Contexts.
Group related apps when their desired dictation behavior is similar; a single
site visit, an installed app, or a guessed work category is not enough to create
its own Context. Explain the evidence, expected benefit and uncertainty for each
suggestion. If evidence is thin, offer a small starting point or ask one focused
question instead of filling a quota. For each recommendation, connect the
observed usage and current assignment to the specific benefit; explicitly say
when current settings already cover the workflow or the evidence supports no
addition. Separate a recommendation from an action,
and ask which changes the user wants before applying them. Use native controls,
read back the results, and report verification limits.
