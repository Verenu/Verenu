# Live model discovery

Settings refreshes cloud model lists daily, after a saved API key, or with
**Refresh models** in either model picker. Provider calls stay in Rust and use
the native credential store. OpenRouter's public list is available before a key
is saved; selecting a model still requires a key.

Groq, OpenAI, Google, OpenRouter, and xAI determine availability through their
model APIs. Google's list is fully paginated. Capability metadata comes from
the public `https://models.dev/api.json?type=all` catalog, with OpenRouter's
architecture and xAI's language-model modalities taking priority. OpenRouter is
queried with `output_modalities=all` so its dedicated speech models are included;
only `transcription` output qualifies for its speech endpoint. Metadata only
classifies model IDs present in the provider response. Compatible discoveries
appear automatically; unknown capabilities remain under **Show more models**.
Audio models that require a different endpoint, such as OpenAI Realtime, are
excluded. Listing and metadata establish compatibility, not dictation quality.

AssemblyAI has no documented discovery API. Its models come from the versioned
[exceptions catalog](../resources/model-catalog.json), also published from the
repository's `master` branch through GitHub's raw endpoint. The catalog supplements
Google's dedicated Interactions transcriber, which is absent from generateContent
discovery. Routine AssemblyAI additions can be published by updating this JSON
on `master`, without shipping another installer. The online file becomes available
when this change is merged; until then, the bundled copy is used.

The exceptions schema accepts labels and the tasks `transcription` and `cleanup`.
It cannot supply endpoint URLs, credentials, code, or prompts. Only AssemblyAI
can add arbitrary undiscovered IDs; Google additions must already appear in its
provider list or be the dedicated transcriber supported by this build. New
endpoint families and request formats still require application changes.

Both IDs and capabilities are cached on the device. Network failures retain the
last list, and failed attempts have a 15-minute retry cooldown. A response that
drops more than half the previous IDs is treated as incomplete. Normal missing
models need two observations at least 15 minutes apart before curated entries
are hidden. Refreshing never changes selected models or fallback order.

Public catalog requests contain no API keys, dictated text, or selected model
IDs. They do reveal the normal network metadata of any HTTPS request to those
hosts. Offline installs retain the bundled models and any cached discoveries.
