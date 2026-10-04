# Privacy & Data

Verenu doesn't run an account system. Configured desktop and Android builds
include optional pseudonymous product analytics, enabled by default. Disable
it during onboarding or in Settings -> Privacy. Dictated content and private
account or device identifiers are excluded; random analytics identifiers are
sent. See the [analytics contract](ANALYTICS.md) for the exact event and identity
rules.

## What stays on your device

- Your API keys in Windows Credential Manager, macOS Keychain, Linux Secret
  Service, or Android Keystore-backed encrypted preferences
- Your settings, provider preferences, context groups, targets, and tone preferences
- Your transcription history
- Your context vocabulary, snippets, and Auto-learn data
- Local logs, unless you explicitly export them

## What leaves your device

- **Recorded audio** goes to the transcription provider you chose when you finish a dictation unless transcription is local
- **Raw transcription text** goes to your chosen cleanup provider if cleanup is enabled
- **Cleanup context** goes along with cleanup requests, including context instructions, cleanup settings, and model metadata
- **Active app context** leaves your device only if you've enabled app-context hints
- **Context website checks** send the domain you typed (nothing else) to DNS when you attach a website to a context group, to confirm it actually exists before saving it
- **Update checks** request GitHub release metadata without sending dictated text, history, or keys
- **Verenu service checks** optionally request public provider status and health data from `api.verenu.com`; disable them in Settings → Privacy

## Optional product analytics

Configured builds send approved usage events, safe settings summaries, and
sanitized failure categories. Analytics uses a persisted random installation
ID, a fresh session ID, and per-dictation run IDs. The installation ID allows
cross-launch usage measurement and is deleted when analytics is disabled.
Re-enabling creates a new ID. These IDs are not derived from accounts,
hardware, pairing identity, or user content. See [Data and privacy](DATA_AND_PRIVACY.md)
and the [analytics contract](ANALYTICS.md) for exclusions and opt-out behavior.

## One important caveat

Once your audio or text reaches a third-party AI provider like Groq, OpenAI, Google, or AssemblyAI, that provider's own retention and privacy policies apply. Verenu has no control over what happens on their end.

For on-device dictation processing, use local transcription with local cleanup
or Cleanup Off. Local transcription with cloud cleanup still sends transcript
text to the cleanup provider. Model downloads, update checks, and optional
analytics are separate network activity.

## Want the full breakdown?

This page covers the essentials. For the full technical breakdown, including a feature-by-feature data map, backup and export contents, and key storage details, see [DATA_AND_PRIVACY.md](DATA_AND_PRIVACY.md).

## Related Docs

<p align="center">
  <a href="DATA_AND_PRIVACY.md"><img alt="Full Privacy Doc" src="https://img.shields.io/badge/Full-Privacy%20Doc-c44632"></a>
  <a href="API_KEYS.md"><img alt="API Keys" src="https://img.shields.io/badge/API-Keys-5b554a"></a>
  <a href="SECURITY.md"><img alt="Security Policy" src="https://img.shields.io/badge/Security-Policy-2b2422"></a>
</p>
