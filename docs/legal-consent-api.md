# Legal consent API

Implementation in progress: `get_install_id`, `get_consent_state`,
`skip_legal`, and `accept_legal` are registered. Skip does not yet gate all
provider calls, and deletion is not implemented. Acceptance and sync are not
yet verified end to end.

The app invokes these Tauri commands with `invoke`. Both current document
versions are `2026-11-03`; their public URLs are
`https://verenu.com/legal/terms/2026-11-03` and
`https://verenu.com/legal/privacy/2026-11-03`.

```ts
type ConsentState = {
  status: 'unanswered' | 'skipped' | 'accepted' | 'update_required' | 'deletion_pending';
  termsVersion: string;
  privacyVersion: string;
  acceptedAt: number | null; // Unix milliseconds
  syncPending: boolean;
};

invoke<ConsentState>('get_consent_state');
invoke<ConsentState>('accept_legal');
invoke<ConsentState>('skip_legal');
invoke<ConsentState>('delete_my_data');
invoke<string>('get_install_id');
```

All commands take no arguments. They reject with an error string when local
storage fails. An acceptance can succeed offline with `syncPending: true`.
Deletion may return `deletion_pending` until the server confirms deletion.
The installation secret is never returned to the frontend. A copied ID is a
support handle and does not authorize access or deletion.
