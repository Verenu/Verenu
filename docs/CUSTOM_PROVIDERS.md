# Custom providers

Open **Settings > API Keys > Add custom provider** in the desktop app. Pick a preset or **Start from scratch**. Presets cover popular cloud APIs, Anthropic-style endpoints, and local servers such as Ollama, LM Studio, llama.cpp, and vLLM. They only prefill the base URL, protocol, capabilities, and a suggested model ID; click a suggestion chip to add more. Verenu does not test or support these vendors, and model IDs change, so check the vendor's docs. Give the provider a name, choose a protocol, and enter its base URL. Include the API version prefix, such as `https://api.example.com/v1`. Verenu appends the task path.

| Protocol | Transcription path | Cleanup path |
| --- | --- | --- |
| OpenAI compatible | `/audio/transcriptions`, multipart audio | `/chat/completions` |
| Anthropic compatible | Not supported | `/messages` |
| xAI compatible | `/stt`, multipart audio | `/chat/completions` |

Enter one model ID per line for each enabled task. These IDs belong to your endpoint. Verenu does not discover or verify their capabilities. Choose them in **Settings > Models > Advanced Models > Change model**, where they appear under your provider's name. They can also be used as fallbacks.

Add the API key while saving the provider, or edit the provider to add it later. A failed `/models` check can be inconclusive when the endpoint does not implement model listing. A definitive authentication rejection prevents saving the key. Keys remain in the operating system's credential store and are never returned to the UI. Disable **Requires API key** for an endpoint that accepts requests without authentication.

**Advanced request settings** allows a custom API key header, non-secret extra headers as a JSON object with string values, and extra JSON fields in cleanup requests. A custom key header receives the raw key. Leave it blank for bearer authentication on OpenAI and xAI compatible endpoints, or `x-api-key` on Anthropic compatible endpoints. Do not put credentials in extra headers. Core request fields, including model, messages, system prompt, token limits, tools, and streaming, cannot be overridden.

The editor displays the destination receiving your dictated audio and text. HTTPS is required for public endpoints. Plain HTTP is allowed for localhost and private network addresses. Requests never follow redirects and custom JSON responses are limited to 4 MB. Custom endpoint error bodies are discarded without logging their contents.

Renaming keeps the provider's identity and saved key. Changing the base URL, protocol, or key header requires entering the key again when one is already saved. Removing a provider clears its key, removes its selected models, and promotes the next fallback or the standard Groq model.

Provider definitions are included in settings backups without keys. They do not sync over LAN. This editor supports the three protocols above; arbitrary request templates and code-block import are separate future work.
