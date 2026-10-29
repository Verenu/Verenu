<script lang="ts">
  import { onMount } from 'svelte';
  import { slide } from 'svelte/transition';
  import { formatIpcError } from '../../errors';
  import { MOTION_MS, motionMs } from '../../motion';
  import { customProviderStore, refreshCustomProviders, type CustomProvider } from '../../customProviders.svelte';
  import { presetForUrl, displayHost } from '../../customProviderPresets';
  import { presetLogoHtml } from '../../customProviderLogos';
  import { invoke } from '../../tauri';
  import { isAndroid } from '../../platform';
  import { getProviderLogo, getProviderPlate } from '../../setup/ProviderLogos';
  import { refreshCatalog } from '../../modelCatalogStore.svelte';
  import { BUILTIN_PROVIDERS, type BuiltinProvider } from './builtinProviders';
  import ProviderCaps from './ProviderCaps.svelte';
  import ProviderDrawer from './ProviderDrawer.svelte';
  import ProviderTile from './ProviderTile.svelte';

  const MAX_CUSTOM = 12;

  let drawer = $state<ProviderDrawer | null>(null);
  let loaded = $state(false);
  let keyStatus = $state<Record<string, boolean>>({});
  let confirming = $state<string | null>(null);
  let busy = $state(false);
  let error = $state('');
  let notice = $state('');
  let noticeTimer: ReturnType<typeof setTimeout> | undefined;

  const addedBuiltins = $derived(BUILTIN_PROVIDERS.filter(p => keyStatus[p.id]));
  const customs = $derived(customProviderStore.providers);
  const count = $derived(addedBuiltins.length + customs.length);
  const atLimit = $derived(customs.length >= MAX_CUSTOM);

  function flash(message: string) {
    notice = message;
    clearTimeout(noticeTimer);
    noticeTimer = setTimeout(() => (notice = ''), 6000);
  }

  async function loadKeyStatus() {
    try {
      let status = await invoke<Record<string, boolean>>('get_api_key_status');
      if (isAndroid) {
        try {
          const durable = await invoke<Record<string, boolean>>('plugin:verenu-security|getCredentialStatus');
          status = { ...status, ...durable };
        } catch (err) {
          console.warn('native credential status unavailable:', err);
        }
      }
      keyStatus = status;
    } catch (err) {
      console.error('get_api_key_status failed:', err);
    }
    return keyStatus;
  }

  async function refresh() {
    await Promise.all([loadKeyStatus(), isAndroid ? Promise.resolve() : refreshCustomProviders()]);
  }

  async function saveBuiltinKey(provider: BuiltinProvider['id'], raw: string): Promise<{ error?: string; notice?: string }> {
    const key = raw.trim();
    if (!key) return { error: 'Enter an API key.' };
    let validation: { status: 'valid' | 'invalid' | 'unknown'; message: string };
    try {
      const result = await invoke<{ status: 'valid' | 'invalid' | 'unknown'; message: string }>('validate_api_key', { provider, key });
      validation = { status: result.status, message: result.message };
    } catch (err) {
      validation = { status: 'unknown', message: formatIpcError(err, 'Could not verify this API key') };
    }
    if (validation.status === 'invalid') return { error: validation.message || 'The provider rejected this key.' };

    try {
      if (isAndroid) {
        try {
          await invoke('plugin:verenu-security|saveCredential', { provider, key });
        } catch (error) {
          console.warn('direct Android credential save unavailable; using bridge fallback', error);
        }
      }
      await invoke('save_api_key', { provider, key });
      if (isAndroid) await invoke('android_keystore_save', { provider, key });
      const status = await loadKeyStatus();
      if (!status[provider]) return { error: 'Verenu could not confirm that this key was saved. Unlock your system key storage and try again.' };
      window.dispatchEvent(new CustomEvent('verenu:api-key-saved', { detail: { provider } }));
      void refreshCatalog(provider, []);
      return validation.status === 'valid'
        ? { notice: 'Key verified and saved.' }
        : { notice: 'Key saved, but it could not be verified.' };
    } catch (e) {
      console.error('save_api_key failed', e);
      return { error: formatIpcError(e, 'Could not save this API key on this device') };
    }
  }

  async function removeBuiltin(provider: BuiltinProvider) {
    busy = true; error = '';
    try {
      if (isAndroid) {
        try {
          await invoke('plugin:verenu-security|saveCredential', { provider: provider.id, key: '' });
        } catch (err) {
          console.warn('direct Android credential delete unavailable; using bridge fallback', err);
        }
      }
      await invoke('delete_api_key', { provider: provider.id });
      if (isAndroid) await invoke('android_keystore_save', { provider: provider.id, key: '' });
      await loadKeyStatus();
      confirming = null;
      window.dispatchEvent(new CustomEvent('verenu:api-key-deleted', { detail: { provider: provider.id } }));
      flash('Provider removed.');
    } catch (e) {
      console.error('delete_api_key failed', e);
      error = formatIpcError(e, 'Could not remove this API key from this device');
    } finally { busy = false; }
  }

  async function removeCustom(provider: CustomProvider) {
    busy = true; error = '';
    try {
      await invoke('delete_custom_provider', { provider: provider.id });
      await refresh();
      confirming = null;
      flash('Provider removed. Its models were taken out of your fallback chains.');
    } catch (e) {
      error = formatIpcError(e, 'Could not remove this provider');
    } finally { busy = false; }
  }

  onMount(() => {
    void refresh().catch(e => (error = formatIpcError(e, 'Could not load providers'))).finally(() => (loaded = true));
    return () => clearTimeout(noticeTimer);
  });
</script>

{#snippet trash()}<svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"><path d="M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l.6 8h5.8l.6-8" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/></svg>{/snippet}
{#snippet pencil()}<svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"><path d="M10.5 3l2.5 2.5L6 12.5 3 13l.5-3z" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round"/></svg>{/snippet}

<div class="page-head">
  <h2 class="settings-h page-title">Providers</h2>
  <button class="btn-primary btn-compact add" type="button" data-setting-target="custom-providers" disabled={atLimit} onclick={() => drawer?.openPicker()}>
    <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true"><path d="M8 3v10M3 8h10" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" fill="none"/></svg>
    Add provider
  </button>
</div>
<p class="panel-note">{isAndroid ? 'Keys are encrypted in the Android Keystore.' : 'Keys stay on this device and are never shown again.'}</p>

{#if count}
  <ul class="list" aria-label="Your providers">
    {#each addedBuiltins as item (item.id)}
      <li class="row" data-setting-target={`api-key-${item.id}`}>
        <ProviderTile name={item.label} logo={getProviderLogo(item.id)} plate={getProviderPlate(item.id)} />
        <div class="info"><span class="name">{item.label}</span></div>
        <ProviderCaps transcription={item.transcription} cleanup={item.cleanup} />
        <div class="actions">
          <button type="button" class="icon-btn" aria-label={`Replace ${item.label} key`} title="Replace key" disabled={busy} onclick={() => drawer?.edit({ kind: 'builtin', id: item.id })}>{@render pencil()}</button>
          <button type="button" class="icon-btn danger" aria-label={`Remove ${item.label}`} title="Remove" disabled={busy} onclick={() => (confirming = item.id)}>{@render trash()}</button>
        </div>
        {#if confirming === item.id}
          <div class="confirm" transition:slide={{ duration: motionMs(MOTION_MS.fast) }}>
            <span>Remove the saved {item.label} key?</span>
            <button class="btn-danger btn-compact" type="button" disabled={busy} onclick={() => removeBuiltin(item)}>Remove</button>
            <button class="btn-ghost btn-compact" type="button" disabled={busy} onclick={() => (confirming = null)}>Cancel</button>
          </div>
        {/if}
      </li>
    {/each}
    {#each customs as provider (provider.id)}
      {@const match = presetForUrl(provider.base_url, provider.protocol)}
      <li class="row" data-setting-target={`api-key-${provider.id}`}>
        <ProviderTile name={provider.name} logo={presetLogoHtml(match?.id)} color={match?.color} />
        <div class="info">
          <span class="name">{provider.name}
            {#if provider.requires_key && !keyStatus[provider.id]}<span class="pill warn">Needs key</span>{/if}
          </span>
          <span class="sub">{displayHost(provider.base_url)}</span>
        </div>
        <ProviderCaps transcription={provider.supports_transcription} cleanup={provider.supports_cleanup} />
        <div class="actions">
          <button type="button" class="icon-btn" aria-label={`Edit ${provider.name}`} title="Edit" disabled={busy} onclick={() => drawer?.edit({ kind: 'custom', provider })}>{@render pencil()}</button>
          <button type="button" class="icon-btn danger" aria-label={`Remove ${provider.name}`} title="Remove" disabled={busy} onclick={() => (confirming = provider.id)}>{@render trash()}</button>
        </div>
        {#if confirming === provider.id}
          <div class="confirm" transition:slide={{ duration: motionMs(MOTION_MS.fast) }}>
            <span>Remove {provider.name} and its saved key?</span>
            <button class="btn-danger btn-compact" type="button" disabled={busy} onclick={() => removeCustom(provider)}>Remove provider</button>
            <button class="btn-ghost btn-compact" type="button" disabled={busy} onclick={() => (confirming = null)}>Cancel</button>
          </div>
        {/if}
      </li>
    {/each}
  </ul>
{:else if loaded}
  <div class="empty">
    <span class="empty-icon" aria-hidden="true">
      <svg viewBox="0 0 24 24" width="22" height="22"><path d="M12 3v4M12 17v4M3 12h4M17 12h4M6 6l2.5 2.5M15.5 15.5L18 18M18 6l-2.5 2.5M8.5 15.5L6 18" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/></svg>
    </span>
    <p class="empty-title">No providers yet</p>
    <button class="btn-primary btn-compact" type="button" onclick={() => drawer?.openPicker()}>Choose a provider</button>
  </div>
{/if}

{#if error}<p class="error" role="alert">{error}</p>{/if}
{#if notice}<p class="notice" role="status" transition:slide={{ duration: motionMs(MOTION_MS.fast) }}>{notice}</p>{/if}

<p class="fineprint">Logos belong to their owners. Verenu is not affiliated with these companies.</p>

<ProviderDrawer
  bind:this={drawer}
  builtins={BUILTIN_PROVIDERS}
  {keyStatus}
  allowCustom={!isAndroid}
  {saveBuiltinKey}
  onchange={refresh}
  onnotice={flash}
/>

<style>
  .page-head { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: var(--settings-h-mb, 20px); }
  .page-head h2.page-title { margin: 0; }
  .add { display: inline-flex; align-items: center; gap: 6px; flex: none; }

  .list { list-style: none; margin: 4px 0 0; padding: 0; display: grid; gap: 8px; }
  .row { display: flex; align-items: center; flex-wrap: wrap; gap: 12px; padding: 10px 12px; border: 1px solid var(--line); border-radius: 14px; background: var(--bg-elev); transition: border-color 140ms ease, background 140ms ease; }
  .row:hover { border-color: var(--line-strong); }
  .info { flex: 1; min-width: 0; display: grid; gap: 2px; }
  .name { display: flex; align-items: center; gap: 8px; font-size: 13.5px; font-weight: 600; color: var(--ink); }
  .sub { font-size: 11.5px; color: var(--ink-mute); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .pill { font-size: 10px; font-weight: 600; padding: 2px 8px; border-radius: 999px; }
  .pill.warn { background: var(--warning-bg); color: var(--warning); }
  .actions { display: flex; gap: 2px; flex: none; }
  .icon-btn { width: 30px; height: 30px; display: grid; place-items: center; padding: 0; border: 0; border-radius: 9px; background: transparent; color: var(--ink-mute); cursor: pointer; transition: background 120ms ease, color 120ms ease, transform 120ms ease; }
  .icon-btn:hover:not(:disabled) { background: var(--control-hover); color: var(--ink); }
  .icon-btn.danger:hover:not(:disabled) { background: var(--danger-bg); color: var(--danger); }
  .icon-btn:active:not(:disabled) { transform: scale(0.92); }
  .icon-btn:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .icon-btn:disabled { opacity: 0.4; cursor: default; }
  .confirm { width: 100%; display: flex; align-items: center; gap: 8px; flex-wrap: wrap; padding-top: 10px; border-top: 1px solid var(--line-soft); font-size: 12px; color: var(--ink-soft); }
  .confirm span { flex: 1 1 200px; }

  .empty { display: grid; justify-items: center; gap: 10px; padding: 44px 16px; border: 1px dashed var(--line); border-radius: 16px; text-align: center; }
  .empty-icon { display: grid; place-items: center; width: 44px; height: 44px; border-radius: 14px; background: var(--paper-2); color: var(--ink-mute); }
  .empty-title { margin: 0; font-size: 13.5px; font-weight: 600; color: var(--ink); }

  .error { margin: 12px 0 0; color: var(--danger); font-size: 12px; }
  .notice { margin: 12px 0 0; padding: 9px 12px; border-radius: 10px; font-size: 12px; background: var(--success-bg); color: var(--ink); }
  .fineprint { margin: 18px 0 0; font-size: 11px; color: var(--ink-faint); }

  @media (prefers-reduced-motion: reduce) { .icon-btn, .row { transition: none; } }
</style>
