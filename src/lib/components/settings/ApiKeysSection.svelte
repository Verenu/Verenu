<script lang="ts">
  import { formatIpcError } from '../../errors';
  import CustomProvidersSection from './CustomProvidersSection.svelte';
  import { customProviderStore } from '../../customProviders.svelte';
  import { onMount } from 'svelte';
  import { invoke } from '../../tauri';
  import { isAndroid } from '../../platform';
  import { getProviderLogo } from '../../setup/ProviderLogos';
  import { refreshCatalog } from '../../modelCatalogStore.svelte';

  type ProviderId = 'groq' | 'openai' | 'google' | 'assemblyai' | 'openrouter' | 'xai';
  type KeyStatus = Record<ProviderId, boolean>;
  type KeyDrafts = Record<ProviderId, string>;
  type KeyValidation = { status: 'idle' | 'checking' | 'valid' | 'invalid' | 'unknown'; message: string };

  const keyProviders: { id: ProviderId; label: string; ph: string; models: string }[] = [
    { id: 'groq',       label: 'Groq',       ph: 'gsk_…',        models: 'Transcription and cleanup' },
    { id: 'openai',     label: 'OpenAI',     ph: 'sk-…',         models: 'Transcription and cleanup' },
    { id: 'google',     label: 'Gemini',     ph: 'AIza…',        models: 'Transcription and cleanup' },
    { id: 'assemblyai', label: 'AssemblyAI', ph: '32-char key',  models: 'Transcription' },
    { id: 'openrouter', label: 'OpenRouter', ph: 'sk-or-…',      models: 'Hundreds of models through one key' },
    { id: 'xai',        label: 'xAI',        ph: 'xai-…',        models: 'Transcription and cleanup' },
  ];

  const perProvider = <T,>(value: () => T) =>
    Object.fromEntries(keyProviders.map((p) => [p.id, value()])) as Record<ProviderId, T>;

  let addRequest = $state(0);
  let keyStatus = $state<KeyStatus>(perProvider(() => false));
  let draftKeys = $state<KeyDrafts>(perProvider(() => ''));
  let keySaving = $state<Record<ProviderId, boolean>>(perProvider(() => false));
  let keyErrors = $state<KeyDrafts>(perProvider(() => ''));
  let keyValidation = $state<Record<ProviderId, KeyValidation>>(
    perProvider<KeyValidation>(() => ({ status: 'idle', message: '' })),
  );

  async function loadKeyStatus() {
    try {
      let status = await invoke<KeyStatus>('get_api_key_status');
      if (isAndroid) {
        try {
          // Rust's Android cache is intentionally memory-only and may be empty
          // until AccessibilityService reconnects. Ask the native Keystore for
          // booleans only so a durable key does not look lost in Settings.
          const durable = await invoke<Partial<KeyStatus>>('plugin:verenu-security|getCredentialStatus');
          status = { ...status, ...durable };
        } catch (err) {
          // The Rust status is still useful if an older build lacks the native
          // command or the Keystore is temporarily unavailable.
          console.warn('native credential status unavailable:', err);
        }
      }
      keyStatus = status;
      return status;
    } catch (err) {
      console.error('get_api_key_status failed:', err);
      return keyStatus;
    }
  }

  // Save now tests first: a definitively-rejected key (401/403) is never persisted.
  async function saveKey(provider: ProviderId) {
    const key = draftKeys[provider].trim();
    if (!key) return;
    keyErrors[provider] = '';
    keyValidation[provider] = { status: 'checking', message: '' };
    keySaving[provider] = true;
    try {
      let validation: { status: 'valid' | 'invalid' | 'unknown'; message: string };
      try {
        const result = await invoke<{ ok: boolean; status: 'valid' | 'invalid' | 'unknown'; message: string }>('validate_api_key', { provider, key });
        validation = { status: result.status, message: result.message };
      } catch (err) {
        validation = { status: 'unknown', message: formatIpcError(err, 'Could not verify this API key') };
      }

      // Definitive rejection — don't store it, surface the failure in red.
      if (validation.status === 'invalid') {
        keyValidation[provider] = validation;
        return;
      }

      if (isAndroid) {
        // Persist directly through the native Keystore plugin so this does
        // not depend on AccessibilityService being enabled and connected.
        // Rust's Android command remains the memory-only pipeline cache.
        try {
          await invoke('plugin:verenu-security|saveCredential', { provider, key });
        } catch (error) {
          console.warn('direct Android credential save unavailable; using bridge fallback', error);
        }
      }
      await invoke('save_api_key', { provider, key });
      if (isAndroid) await invoke('android_keystore_save', { provider, key });
      const status = await loadKeyStatus();
      if (!status[provider]) {
        keyValidation[provider] = { status: 'idle', message: '' };
        keyErrors[provider] = 'Verenu could not confirm that this key was saved. Unlock your system key storage, then save the key again.';
        return;
      }

      draftKeys[provider] = '';
      // The model picker lives in a sibling section and can only list a
      // provider's models once the key is actually saved, so tell it now
      // rather than making the user reopen Settings.
      window.dispatchEvent(new CustomEvent('verenu:api-key-saved', { detail: { provider } }));
      void refreshCatalog(provider, []);
      // 'unknown' = couldn't reach the provider; we saved it anyway (might be fine)
      // but say so plainly instead of claiming it's verified.
      keyValidation[provider] =
        validation.status === 'valid'
          ? { status: 'valid', message: 'Key verified.' }
          : { status: 'unknown', message: `Saved on this device. ${validation.message || 'The provider could not verify it. Check your connection and try verifying it again.'}` };
    } catch (e) {
      console.error('save_api_key failed', e);
      keyValidation[provider] = { status: 'idle', message: '' };
      // Credential-store errors never include the key. Surface the native
      // cause so Linux users can unlock/start Secret Service instead of being
      // sent through an unhelpful generic retry loop.
      keyErrors[provider] = formatIpcError(e, 'Could not save this API key on this device');
    } finally {
      keySaving[provider] = false;
    }
  }

  async function clearKey(provider: ProviderId) {
    keyErrors[provider] = '';
    keySaving[provider] = true;
    try {
      if (isAndroid) {
        try {
          await invoke('plugin:verenu-security|saveCredential', { provider, key: '' });
        } catch (error) {
          console.warn('direct Android credential delete unavailable; using bridge fallback', error);
        }
      }
      await invoke('delete_api_key', { provider });
      if (isAndroid) {
        // Rotate the deletion through Rust's memory cache too.
        await invoke('android_keystore_save', { provider, key: '' });
      }
      await loadKeyStatus();
      draftKeys[provider] = '';
      keyValidation[provider] = { status: 'idle', message: '' };
      window.dispatchEvent(new CustomEvent('verenu:api-key-deleted', { detail: { provider } }));
    } catch (e) {
      console.error('delete_api_key failed', e);
      keyErrors[provider] = formatIpcError(e, 'Could not remove this API key from this device');
    } finally {
      keySaving[provider] = false;
    }
  }

  onMount(() => {
    void loadKeyStatus();
  });
</script>

<div class="page-head">
  <h2 class="settings-h page-title">Providers</h2>
  {#if !isAndroid}
    <button class="add-provider" type="button" data-setting-target="custom-providers" aria-label="Create custom provider" aria-describedby="add-provider-tip"
      disabled={customProviderStore.providers.length >= 12} onclick={() => addRequest += 1}>
      <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"><path d="M8 3v10M3 8h10" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" fill="none"/></svg>
      <span class="add-tip" id="add-provider-tip" role="tooltip">Create custom provider</span>
    </button>
  {/if}
</div>
<p class="panel-note">{isAndroid ? 'Keys are stored encrypted in the Android Keystore and never readable from the UI after saving.' : 'Keys are stored locally and never readable from the UI after saving.'}</p>

{#each keyProviders as item}
  <div class="setting-row key-row" data-setting-target={`api-key-${item.id}`}>
    <div class="key-left">
      <div class="label">
        <span class="key-logo">{@html getProviderLogo(item.id)}</span>
        {item.label}
        {#if keyValidation[item.id].status === 'invalid'}
          <span class="key-status failed" title={keyValidation[item.id].message}>
            <span class="key-status-dot"></span>failed
          </span>
        {:else if keyStatus[item.id]}
          <span class="key-status saved" title={keyValidation[item.id].message || 'Key saved'}>
            <span class="key-status-dot"></span>saved
          </span>
        {/if}
      </div>
      <div class="desc">{item.models}</div>
    </div>
    <div class="key-right">
      <input
        type="password"
        class="key-input"
        aria-label={`${item.label} API key`}
        class:failed={keyValidation[item.id].status === 'invalid'}
        placeholder={keyStatus[item.id] ? '••••••••••••' : item.ph}
        bind:value={draftKeys[item.id]}
        oninput={() => {
          if (keyValidation[item.id].status === 'invalid') keyValidation[item.id] = { status: 'idle', message: '' };
          if (keyErrors[item.id]) keyErrors[item.id] = '';
        }}
        onkeydown={(e) => e.key === 'Enter' && saveKey(item.id)}
        autocomplete="off"
        aria-invalid={keyErrors[item.id] || keyValidation[item.id].status === 'invalid' ? 'true' : 'false'}
      />
      <div class="flip-btn" class:flipped={keyStatus[item.id] && !draftKeys[item.id].trim()}>
        <button
          class="btn-ghost flip-face front"
          onclick={() => saveKey(item.id)}
          disabled={!draftKeys[item.id].trim() || keySaving[item.id]}
          tabindex={keyStatus[item.id] && !draftKeys[item.id].trim() ? -1 : 0}
          aria-hidden={keyStatus[item.id] && !draftKeys[item.id].trim() ? 'true' : 'false'}
        >{keySaving[item.id] ? 'Saving…' : 'Save'}</button>
        <button
          class="btn-ghost btn-clear flip-face back"
          onclick={() => clearKey(item.id)}
          disabled={!keyStatus[item.id] || draftKeys[item.id].trim().length > 0 || keySaving[item.id]}
          tabindex={keyStatus[item.id] && !draftKeys[item.id].trim() ? 0 : -1}
          aria-hidden={keyStatus[item.id] && !draftKeys[item.id].trim() ? 'false' : 'true'}
        >Clear</button>
      </div>
    </div>
    {#if keyErrors[item.id]}
      <p class="key-error">{keyErrors[item.id]}</p>
    {/if}
  </div>
{/each}

<p class="trademark-note">
  The logos above belong to their respective companies. Verenu is not affiliated with, endorsed by, or sponsored by Groq, OpenAI, Google, AssemblyAI, OpenRouter, or xAI — they are shown solely to indicate provider compatibility.
</p>

{#if !isAndroid}<CustomProvidersSection {addRequest} />{/if}

<style>
  .page-head { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: var(--settings-h-mb, 20px); }
  .page-head h2.page-title { margin: 0; }
  .add-provider { position: relative; flex: none; width: 28px; height: 28px; display: grid; place-items: center; padding: 0; border: 1px solid transparent; border-radius: 8px; background: transparent; color: var(--ink-mute); cursor: pointer; transition: background 120ms ease, color 120ms ease, border-color 120ms ease, transform 120ms ease; }
  .add-provider:hover:not(:disabled), .add-provider:focus-visible { background: var(--control-hover); color: var(--ink); border-color: var(--line); }
  .add-provider:active:not(:disabled) { transform: scale(0.92); }
  .add-provider:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .add-provider:disabled { opacity: 0.35; cursor: default; }
  .add-tip { position: absolute; top: calc(100% + 8px); right: 0; z-index: 5; padding: 5px 9px; border-radius: 7px; background: var(--ink); color: var(--paper); font-size: 11px; font-weight: 500; white-space: nowrap; pointer-events: none; opacity: 0; transform: translateY(-3px); transition: opacity 140ms ease, transform 140ms ease; }
  .add-provider:hover:not(:disabled) .add-tip, .add-provider:focus-visible .add-tip { opacity: 1; transform: none; transition-delay: 220ms; }
  @media (prefers-reduced-motion: reduce) { .add-tip, .add-provider { transition: none; } }
  .trademark-note {
    font-size: 11px;
    color: var(--ink-faint);
    line-height: 1.5;
    margin: 14px 0 0;
  }

  /* Centered rather than top-aligned: the row no longer wraps at the widths the
     settings column actually reaches, so flex-start just read as top-heavy. */
  .key-row { align-items: center; gap: 12px; flex-wrap: wrap; }
  .key-left { flex: 1; min-width: 0; }
  .key-logo {
    display: inline-flex;
    width: 16px;
    height: 16px;
    color: var(--ink-mute);
    vertical-align: -3px;
    margin-right: 6px;
  }
  .key-logo :global(svg) { width: 100%; height: 100%; }
  .key-right { display: flex; gap: 6px; align-items: center; flex-shrink: 0; }
  /* Inline status chip next to the provider name — saved (green) / failed (red).
     Slides + pops in; the failed dot pulses a ring twice to draw the eye. */
  .key-status {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    margin-left: 8px;
    font-family: var(--mono);
    font-size: 10px;
    font-weight: 400;
    letter-spacing: 0.02em;
    animation: status-in 0.26s cubic-bezier(0.22, 1, 0.36, 1) both;
  }
  .key-status.saved { color: var(--success); }
  .key-status.failed { color: var(--danger); }
  .key-status-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: currentColor;
    flex-shrink: 0;
    animation: dot-pop 0.3s cubic-bezier(0.22, 1, 0.36, 1) both;
  }
  .key-status.failed .key-status-dot {
    animation:
      dot-pop 0.3s cubic-bezier(0.22, 1, 0.36, 1) both,
      dot-pulse 1.1s ease-out 0.18s 2;
  }
  @keyframes status-in {
    from { opacity: 0; transform: translateX(-5px); }
    to { opacity: 1; transform: none; }
  }
  @keyframes dot-pop {
    from { transform: scale(0); }
    60% { transform: scale(1.3); }
    to { transform: scale(1); }
  }
  @keyframes dot-pulse {
    0% { box-shadow: 0 0 0 0 color-mix(in srgb, var(--danger) 50%, transparent); }
    70% { box-shadow: 0 0 0 5px color-mix(in srgb, var(--danger) 0%, transparent); }
    100% { box-shadow: 0 0 0 0 color-mix(in srgb, var(--danger) 0%, transparent); }
  }
  .key-input {
    font-family: var(--mono);
    font-size: 11.5px;
    background: transparent;
    border: 1px solid var(--line);
    padding: 5px 9px;
    border-radius: 6px;
    color: var(--ink-soft);
    width: clamp(200px, 40cqi, 320px);
    letter-spacing: 0.04em;
  }
  .key-input::-ms-reveal {
    display: none;
  }

  .key-input::-ms-clear {
    display: none;
  }

  .key-input::-webkit-credentials-auto-fill-button {
    display: none;
  }

  .key-input::-webkit-contacts-auto-fill-button {
    display: none;
  }
  .key-input:focus {
    outline: none;
    border-color: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 15%, transparent);
  }
  .btn-clear {
    color: var(--danger);
    border-color: var(--danger-line);
  }
  .btn-clear:hover {
    background: var(--danger-bg);
    border-color: var(--danger-line);
  }

  /* Both actions share one grid cell to keep the button width stable.
     Visibility prevents disabled-button opacity rules revealing the inactive face. */
  .flip-btn {
    display: inline-grid;
    min-width: 72px;
    flex-shrink: 0;
    position: relative;
  }
  .flip-btn.flipped { min-width: 72px; }
  .flip-face {
    grid-area: 1 / 1;
    width: 100%;
    text-align: center;
    transition: opacity 160ms var(--ui-ease-out);
  }
  .flip-face.front { visibility: visible; pointer-events: auto; }
  /* `:disabled` is listed too: `.settings-body .btn-ghost:disabled` sets its
     own opacity and would otherwise leave both labels visible on top of each
     other. */
  .flip-btn .flip-face.back,
  .flip-btn .flip-face.back:disabled { visibility: hidden; opacity: 0; pointer-events: none; }
  .flip-btn.flipped .flip-face.front { visibility: hidden; opacity: 0; pointer-events: none; }
  .flip-btn.flipped .flip-face.back { visibility: visible; opacity: 1; pointer-events: auto; }

  /* Failure feedback: red border + one-shot shake when a key is rejected. */
  .key-input[aria-invalid='true'] { border-color: var(--danger); }
  .key-input.failed {
    border-color: var(--danger);
    animation: key-shake 0.4s ease;
  }
  .key-input.failed:focus {
    border-color: var(--danger);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--danger) 18%, transparent);
  }
  @keyframes key-shake {
    0%, 100% { transform: translateX(0); }
    20% { transform: translateX(-4px); }
    40% { transform: translateX(4px); }
    60% { transform: translateX(-3px); }
    80% { transform: translateX(2px); }
  }

  .key-error {
    width: 100%;
    margin: 4px 0 0;
    font-size: 11px;
    color: var(--danger);
  }

  @container settings-panel (max-width: 520px) {
    .key-row {
      align-items: flex-start;
    }

    .key-right {
      width: 100%;
      justify-content: flex-start;
      flex-wrap: wrap;
    }

    .key-input {
      width: min(100%, 320px);
      flex: 1 1 220px;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .flip-face { transition: none; }
    .key-input.failed,
    .key-status,
    .key-status-dot { animation: none; }
  }
</style>
