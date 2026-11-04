# Legal consent API

Implementation in progress: `get_install_id`, `get_consent_state`,
`skip_legal`, `accept_legal`, and `delete_my_data` are registered. Skip does
not yet have end-to-end verification for every provider. Deletion recovery is
under development and has not been verified end to end on Android or desktop.
Acceptance and sync are not yet verified end to end.

Network-dependent operations must reject with the string `consent_required`
when current consent is absent. The backend implementation is incomplete;
this is the target contract, not a guarantee that every path is gated.
An authenticated request to delete an existing Convex record is permitted
without current consent, including while a deletion is pending. An installation
with a recorded never-accepted state deletes locally without a request. Older
identities without that state send an authenticated deletion request because
their acceptance history is unknown.

Features requiring acceptance include analytics; transcription and cleanup
providers, key validation, and provider model and pricing catalogs; Verenu
health, status, and notices; GitHub and T3 integrations; app updates; model and
runtime downloads; local HTTP model servers; and device sync, discovery,
pairing, and remote muting. Network-enabling settings include `analytics_enabled`,
`sync_enabled`, `sync_muting_enabled`, `verenu_service_checks_enabled`, and
`beta_updates_enabled`, `dual_transcription_enabled`, and `cleanup_enabled`;
network cleanup values in `cleanup_intensity`; non-local models in
`transcription_default_model`, `cleanup_default_model`,
`transcription_fallback_models`, and `cleanup_fallback_models`; non-local `transcription_provider` and
`cleanup_provider` selections, nonempty `github_username`, and nonempty
`custom_providers` are also gated
through `save_setting`, `save_setting_if_unchanged`, and
`save_model_settings_if_unchanged`. `import_data` also rejects backups that
would enable these settings without consent, and `delete_custom_provider`
rejects a change that would select a network provider. Provider selection, downloads, update, sync, GitHub,
T3, and credential-validation commands also need this contract. Their complete
command inventory and backend gates are still being audited.

Known network commands include `save_api_key`, `validate_api_key`, `list_provider_models`,
`get_provider_model_catalog`, `check_provider_status`, `check_provider_status_raw`,
`check_global_message`, `check_verenu_api_health`, `check_connectivity`, `check_domain_exists`,
`download_local_llm_model`, `download_local_llm_runtime`,
`download_local_stt_model`, `check_for_update`, `reinstall_latest_update`,
`nvidia_reuse_install`, `nvidia_reuse_realtime_install`, `install_update`,
`connect_t3`, `pull_t3_skills`, `get_github_commits`,
`get_github_username_suggestion`, `sync_pair_connection`, `sync_start_pairing`,
`sync_respond_to_pairing`, and `sync_now`. Background requests and internal
callers also need gates. This list is not yet verified to be exhaustive.

The app invokes these Tauri commands with `invoke`. The current Terms version
is `2026-11-03` and the Privacy version is `2026-11-03.1`; their public URLs are
`https://verenu.com/legal/terms/2026-11-03` and
`https://verenu.com/legal/privacy/2026-11-03.1`.

```ts
type ConsentState = {
  status: 'unanswered' | 'skipped' | 'accepted' | 'update_required' | 'deletion_pending';
  termsVersion: string;
  privacyVersion: string;
  acceptedAt: number | null; // Unix milliseconds
  syncPending: boolean;
};

invoke<ConsentState>('get_consent_state');
invoke<ConsentState>('accept_legal', { termsVersion: '2026-11-03', privacyVersion: '2026-11-03.1' });
invoke<ConsentState>('skip_legal');
invoke<ConsentState>('delete_my_data');
invoke<string>('get_install_id');
```

The acceptance command requires the versions shown to the user and rejects
outdated versions with an error string starting `consent_version_conflict`,
including when the acceptance endpoint returns HTTP 409. The backend checks
the endpoint before persisting the new acceptance; a 409 clears existing
acceptance and disables gated features. If the endpoint cannot be reached,
acceptance can succeed offline with `syncPending: true`; a later 409 revokes it.
The other commands take no arguments. Commands reject with an error string when local storage fails.
Deletion may return `deletion_pending` until the server confirms deletion.
The installation secret is never returned to the frontend. A copied ID is a
support handle and does not authorize access or deletion.

Browser dev sessions use an isolated synthetic installation secret stored in
their private session directory. They keep acceptance sync pending instead of
writing synthetic acceptance records to the live service.
