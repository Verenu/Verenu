# Product analytics contract

Verenu's desktop and Android product analytics are pseudonymous and enabled by
default. They can be disabled during onboarding or at any time in Settings →
Privacy.
Verenu uses three unrelated random identifiers: a persisted analytics-only
installation ID, a new process/session ID, and a per-dictation run ID. None is
derived from an account, pairing UUID, device, package, network, or user
content. The installation ID is deleted when analytics is disabled and a new
one is generated if analytics is enabled again. This creates continuity for
one installation, not identity for a human; separate devices/installations are
never linked.

Metric terminology is installation-based: a launched installation emits
`app_launched`, an active installation emits `dictation_started`, and a
successful active installation emits `dictation_inserted`. An activated
installation has reached its first successful delivery. Lifetime installation
counts are secondary because opting out deletes identity A and re-enabling
creates identity B; the same physical installation can therefore be counted
twice over its lifetime.

Core definitions: a known installation has emitted any telemetry; a new
installation is first seen through a newly generated analytics identity; a
launched installation emitted `app_launched`; an active installation emitted
`dictation_started`; a successful active installation emitted
`dictation_inserted`; and a retained installation was previously activated and
delivered again in the selected later period. These metrics describe
installations, not people.

The contract is schema version 4. Product code does not call PostHog directly;
all Android events go through `VerenuAnalytics.kt`, and all Windows/macOS
events go through `src-tauri/src/analytics.rs`. Those boundaries own the
allowlists, normalization, opt-out state, identity, and transport.

## Events

| Event | Fires when | Custom properties | Run ID | Funnel use |
| --- | --- | --- | --- | --- |
| `app_launched` | Desktop app startup | `first_launch` | no | activation |
| `setup_started` | Onboarding opens | none | no | setup start |
| `setup_step_viewed` | An onboarding step is shown | `setup_step` | no | setup funnel |
| `setup_step_completed` | User advances from an onboarding step | `setup_step` | no | setup funnel |
| `setup_step_duration` | User leaves a setup step | `setup_step`, `duration_bucket` | no | setup timing |
| `setup_completed` | Setup settings and completion flag save | none | no | activation |
| `setting_changed` | An approved setting changes | `setting`, `value` | no | adoption |
| `settings_snapshot` | Once per desktop process at startup / first Android recording | approved settings below, `context_group_count`, `feature_breadth` | no | configuration context |
| `dictation_started` | Recording start is accepted | `run_id`, `handsfree`, `noise_reduction` | yes | start |
| `recording_finished` | Recording stop is accepted | `run_id`, `recording_duration_bucket` | yes | capture completion |
| `dictation_stopped` | Stop request completes | `run_id` | yes | lifecycle |
| `dictation_cancelled` | User cancellation completes | `run_id` | yes | intentional exit |
| `pipeline_stage_started` | A new visible pipeline stage is observed | `run_id`, `pipeline_stage` | yes | stage timing proxy |
| `pipeline_stage_completed` | A bounded pipeline stage completes | `run_id`, `pipeline_stage`, `duration_bucket`, `duration_ms` (0-300000), optional `provider`, `model` families | yes | performance/model adoption |
| `insertion_attempted` | Delivery is attempted | `run_id` | yes | delivery |
| `dictation_inserted` | Delivery succeeds | `run_id`, `delivery_method`, bounded `word_count` (0-10000) | yes | success/usage scale |
| `dictation_outcome` | One final outcome is known, exactly once per started run | `run_id`, `outcome`, `status`, `reason`, bounded `total_duration_ms`, `recording_duration_ms`, `recording_duration_bucket`, `word_count`, provider/model families, `context_result`, `retry_attempt_bucket`, fallback booleans, `recovered` | yes | canonical delivery denominator |
| `pipeline_failed` | A bounded pipeline failure is observed | `run_id`, `stage`, `category`, optional `provider`, `model` families | yes | reliability/model health |
| `retry_attempted` | A bounded retry is taken | `run_id`, `attempt_bucket`, `retry_reason` | yes | recovery |
| `fallback_used` | A transcription or cleanup fallback is used | `run_id`, `fallback` | yes | recovery |
| `permission_event` | A supported permission request completes | `permission_type`, `permission_status` | no | setup/recovery |
| `input_health` | A coarse capture/input state is observed | `input_outcome` | no | reliability |
| `context_match` | Context resolution finishes | `run_id`, `match_result`, `match_source` | yes | context adoption |
| `usage_milestone_reached` | A successful-dictation count milestone is reached once | `milestone` | no | habit formation |
| `sync_flow`, `sync_started`, `sync_completed`, `sync_failed` | Sync or pairing lifecycle transition | `sync_status` | no | operational health |
| `update_available`, update lifecycle events | A supported update transition occurs | `from_version`, `to_version` | no | release health |
| `$exception` | A controlled application error family is reported | fixed Error Tracking fields below | optional | Error Tracking |

Every event also carries `analytics_schema_version` (integer `4`) and
`identity_schema_version` (integer `2`). The `analytics_install_id` is the
PostHog `distinct_id`, persisted in the stable app-data directory on desktop
and app-private SharedPreferences on Android. `analytics_session_id` is a
fresh process/session ID and `run_id` is a per-dictation ID. The installation
ID survives normal restarts, updates, and settings migrations, but not
uninstall/data deletion or analytics opt-out. `first_seen_version` is stored
once beside the installation ID and does not change on upgrade. Common safe
release metadata is `app_version`, `platform`, `arch`, and `build_channel`
(`debug` or `release`).

The current PostHog product dashboards filter to `analytics_schema_version=4`.
This intentionally leaves pre-schema-4 history out of current outcome and
reliability metrics rather than mixing incompatible payload shapes; the new
tiles become populated as rebuilt beta clients emit the contract.

During the identity migration, an existing valid analytics installation ID is
kept; if it has no prior first-seen sidecar, the first release observing it is
recorded. Verenu does not reconstruct or merge the old process identities.

## Error Tracking

Raw exception capture is disabled. The desktop boundary accepts only a typed
`ErrorReport`; Android accepts only a fixed code and stage. The frontend sends
only the two fixed signals `frontend_handled` and `frontend_unhandled`—never a
JavaScript error object, message, rejection value, URL, or stack.

`$exception` may carry `$exception_fingerprint`, `$issue_name`,
`$issue_description`, `$exception_level`, `$exception_list`, `error_domain`,
`error_code`, `error_stage`, `error_severity`, `handled`, `recovered`,
`recovery_method`, `run_id`, and `error_callsite`. Every string is generated
from a fixed allowlist. The single stack frame is a fixed module/callsite
marker, not a raw native or WebView stack trace. Expected VAD rejection,
permission denial, and user cancellation remain product events, not errors.
Desktop sends these events through PostHog's documented manual Error Tracking
endpoint; ordinary product events remain on the normal capture transport.

Severity policy: `warning` is a handled operational condition, `error` is a
handled terminal path, and `fatal` is an unhandled frontend signal or backend
panic. A panic report is best-effort because an aborting process may exit before
its asynchronous network request completes.

## Allowed values

`pipeline_stage`: `permission`, `capture`, `vad`, `preprocessing`,
`transcription`, `dual_transcription`, `cleanup`, `formatting`, `insertion`,
`clipboard`, `local_model`, `sync`, `unknown`.

`delivery_method`: `direct_insertion`, `clipboard_fallback`, `event_only`,
`unknown`.

`outcome`: `success_clean`, `success_after_retry`,
`success_after_transcription_fallback`, `success_after_cleanup_fallback`,
`success_after_clipboard_fallback`, `rejected_expected`, `cancelled_user`,
`failure_terminal`, `unknown`. Exactly one final `dictation_outcome` is emitted
per run when a final outcome is reached; recovery flags promote a clean success
to the most relevant recovered category.

`dictation_outcome.status`: `success`, `rejected`, `cancelled`, `failure`,
or `unknown`. `dictation_outcome.reason` is either `clean`,
`retry_recovered`, `transcription_fallback_recovered`,
`cleanup_fallback_recovered`, `clipboard_fallback_recovered`,
`user_cancelled`, or one of the bounded failure categories below. Duration
values are clamped to 15 minutes for total processing and 10 minutes for
recording. `word_count` is clamped to 10,000. Provider and model fields are
fixed families and default to `unknown`; they never contain configured model
IDs or endpoints. Fallback booleans describe the run, while `recovered` is
true only for a successful outcome after retry or fallback.

`attempt_bucket`: `1`, `2`, `3`, `4+`.

`fallback`: `transcription`, `cleanup`, `clipboard`, `unknown`.

`match_result`: `matched`, `no_match`, `unknown`; `match_source`:
`automatic`, `manual`, `unknown`.

`milestone`: `dictations_5`, `dictations_10`, `dictations_25`, `unknown`.

`provider`: `groq`, `openai`, `google`, `assemblyai`, `local`, `unknown`.
`model` is a fixed family such as `transcription_whisper`, `google_gemini`,
`groq_qwen`, `chat_model`, `assemblyai_universal`, `local_catalog`, or
`unknown`; raw configured model IDs never leave Verenu. Provider/model fields
are only attached to stage completion/failure events and are derived from the
known pipeline provider string.

`permission_type`: `microphone`, `accessibility`, `notifications`, `battery`,
`unknown`.

`permission_status`: `missing`, `request_shown`, `granted`, `denied`,
`settings_opened`, `recovered`, `still_missing`, `abandoned`, `unknown`.

`duration_bucket`: `<1s`, `1-5s`, `5-15s`, `15-60s`, `60s+`.

`dictation_outcome.context_result`: `matched`, `no_match`, `manual`,
`unknown`; `dictation_outcome.retry_attempt_bucket`: `1`, `2`, `3`, `4+`,
`unknown`.

`recording_duration_bucket`: `under_1s`, `1_5s`, `5_15s`, `15_30s`, `30_60s`,
`60s_plus`.

`input_outcome`: `microphone_available`, `no_input_device`,
`microphone_permission_missing`, `capture_initialized`,
`capture_initialization_failed`, `zero_audio_detected`, `too_quiet`,
`too_short`, `vad_passed`, `vad_rejected`, `vad_internal_failure`,
`capture_stream_failed`, `unknown`.

`word_count` is a bounded locally computed count of whitespace-delimited words
from the successfully processed dictation. It is a coarse output-size metric,
not text, and is never computed remotely or sent with the source string.

Settings properties are only the approved booleans `cleanup_enabled`,
`dual_transcription_enabled`, `noise_reduction`, `mute_audio`, `exclusive_mic`,
`pause_media`, `sound_effects`, `app_context_hint`, `auto_learn_enabled`,
`contextual_formatting`, `contextual_caps`, `auto_spacing`, and
`autostart_enabled`, `mic_mute_button_dictation`, and `sync_enabled`; and the bounded categories `transcription_provider`,
`cleanup_provider`, `cleanup_intensity`, `history_retention`, and
`local_model_memory_policy`. Unknown category values become `unknown`.
`context_group_count` is clamped to 0-200 and excludes the built-in Everywhere
context. `feature_breadth` is a 0-9 count of meaningful capabilities: cleanup,
contexts, auto-learn, dual transcription, local models, contextual formatting,
noise reduction, mute-button dictation, and sync. Neither field contains names
or content.

`usage_milestone_reached` is deliberately low volume. Its local counter is
stored beside the analytics identity and only emits at thresholds 5, 10, and
25 successful deliveries; opt-out deletes that state with the identity.

## Privacy and controls

No event contains audio, transcripts, cleaned text, prompts, snippets,
vocabulary, clipboard contents, focused application/accessibility content,
package or window names, URLs, paths, microphone names, API keys, request or
response bodies, exception text, raw stack traces, or provider model IDs.

Every outbound event includes PostHog processing controls `$ip: "0.0.0.0"`,
`$geoip_disable: true`, and `$process_person_profile: false`. These are added in
the single Rust/Android analytics boundary after the event allowlist, so a new
event cannot omit the GeoIP opt-out. The PostHog project also has the GeoIP
transformation disabled and **Discard client IP data** enabled. The first
control prevents enrichment, while the project setting prevents the connection
IP from being retained. The HTTPS request necessarily reaches PostHog from the
client's network address while analytics is enabled; disabling analytics is the
only way to prevent that network connection.

This is a forward-looking privacy control. Events and person properties stored
before the project transformation was disabled may still contain historical
GeoIP fields. They are not silently rewritten or deleted here because PostHog
does not expose a safe field-level removal operation through the connected
project controls, and deleting persons would also destroy unrelated analytics
history. No location signal replaces GeoIP.

The Settings → Privacy toggle is persisted in Rust settings. It is read by the
Android bridge and applied to every event at the boundary. Disabling opts out,
resets the SDK identity, deletes the durable analytics identity metadata,
clears boundary deduplication state, and prevents new events from being queued.
Events already accepted by the SDK before opt-out may remain in its offline
queue; the SDK opt-out/reset path is invoked immediately, and future events
cannot be emitted while disabled.
Re-enabling creates and persists a new installation identity plus a new session
identity, intentionally breaking continuity across the opt-out boundary. This
is pseudonymous analytics, not a claim of mathematical anonymity.

Dashboard installation metrics use unique PostHog `distinct_id` values rather
than raw event rows. The canonical dictation denominator is one
`dictation_outcome` per `dictation_started` run; lifecycle events are
diagnostic and must not be used as terminal-outcome denominators. Ratio tiles
state their denominator explicitly; settings
distribution tiles use the latest known allowlisted snapshot per installation
within their documented lookback window.

Desktop uses the same PostHog project and safe event contract through the Rust
boundary. Windows and macOS share that implementation, while Android uses the
same event identity model through `VerenuAnalytics`; no platform-specific
identity or content collection is added.

## Release Error Tracking preparation

Controlled `$exception` events remain sanitized before transmission; production
symbol files must never be uploaded from the app. Release tooling may build the
frontend with `VERENU_POSTHOG_SOURCE_MAPS=true`, which emits hidden source maps
without public `sourceMappingURL` comments. A CI-only PostHog Error Tracking
upload step still needs to be wired to the release pipeline with a secret that
never ships in the installer. The current Rust release profile uses symbol
stripping, so native symbolication is not available until that release job
produces and uploads matching unstripped Windows PDB/macOS dSYM artifacts.
The public project ingestion token remains the only PostHog credential allowed
inside a shipped build.
