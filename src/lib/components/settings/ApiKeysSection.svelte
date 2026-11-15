<script module lang="ts">
  // Which providers have a saved key (booleans only), remembered so reopening the section
  // paints the list at once instead of popping it in after the status read.
  let rememberedKeyStatus: Record<string, boolean> | null = null;
</script>

<script lang="ts">
  import { onMount } from 'svelte';
  import { fade, fly, scale, slide } from 'svelte/transition';
  import { flip } from 'svelte/animate';
  import { cubicOut } from 'svelte/easing';
  import Callout from '../Callout.svelte';
  import { formatIpcError } from '../../errors';
  import { listItemCollapse, MOTION_MS, motionMs, motionPx } from '../../motion';
  import { loadSettingsSnapshot } from '../../settings';
  import { customProviderStore, refreshCustomProviders, type CustomProvider } from '../../customProviders.svelte';
  import { presetForUrl, displayHost } from '../../customProviderPresets';
  import { presetLogoHtml } from '../../customProviderLogos';
  import { invoke } from '../../tauri';
  import { handleConsentError } from '../../legal/consentStore.svelte';
  import GatedButton from '../../legal/GatedButton.svelte';
  import ConsentHint from '../../legal/ConsentHint.svelte';
  import { isAndroid } from '../../platform';
  import { getProviderLogo, getProviderPlate } from '../../setup/ProviderLogos';
  import { refreshCatalog } from '../../modelCatalogStore.svelte';
  import { BUILTIN_PROVIDERS, type BuiltinProvider } from './builtinProviders';
  import ProviderCaps from './ProviderCaps.svelte';
  import ProviderDrawer from './ProviderDrawer.svelte';
  import ProviderTile from './ProviderTile.svelte';
  import { providerUse, usageLabel, usageLevel, type UsageSettings } from './providerUsage';

  const MAX_CUSTOM = 12;

  let drawer = $state<ProviderDrawer | null>(null);
  let loaded = $state(rememberedKeyStatus !== null);
  let keyStatus = $state<Record<string, boolean>>(rememberedKeyStatus ?? {});
  let confirming = $state<string | null>(null);
  let busy = $state(false);
  let error = $state('');
  let notice = $state('');
  let noticeTimer: ReturnType<typeof setTimeout> | undefined;

  let usage = $state<UsageSettings>({});

  type Connected = {
    id: string;
    name: string;
    host: string | null;
    logo: string | null;
    plate: 'bleed' | 'none';
    color?: string;
    transcription: boolean;
    cleanup: boolean;
    needsKey: boolean;
    builtin?: BuiltinProvider;
    custom?: CustomProvider;
  };

  const customs = $derived(customProviderStore.providers);
  const connected = $derived<Connected[]>([
    ...BUILTIN_PROVIDERS.filter(p => keyStatus[p.id]).map(p => ({
      id: p.id, name: p.label, host: null, logo: getProviderLogo(p.id), plate: getProviderPlate(p.id),
      transcription: p.transcription, cleanup: p.cleanup, needsKey: false, builtin: p,
    })),
    ...customs.map(p => {
      const match = presetForUrl(p.base_url, p.protocol);
      return {
        id: p.id, name: p.name, host: displayHost(p.base_url), logo: presetLogoHtml(match?.id), plate: 'none' as const, color: match?.color,
        transcription: p.supports_transcription, cleanup: p.supports_cleanup, needsKey: p.requires_key && !keyStatus[p.id], custom: p,
      };
    }),
  ]);
  const available = $derived(BUILTIN_PROVIDERS.filter(p => !keyStatus[p.id]));
  const count = $derived(connected.length);
  const atLimit = $derived(customs.length >= MAX_CUSTOM);
  const allowCustom = $derived(!isAndroid && !atLimit);

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
      rememberedKeyStatus = status;
    } catch (err) {
      console.error('get_api_key_status failed:', err);
    }
    return keyStatus;
  }

  /** Which provider runs which task. The list still works when this read fails. */
  async function loadUsage() {
    try { usage = await loadSettingsSnapshot(); }
    catch (err) { console.warn('provider usage unavailable:', err); }
  }

  async function refresh() {
    await Promise.all([loadKeyStatus(), isAndroid ? Promise.resolve() : refreshCustomProviders(), loadUsage()]);
  }

  function edit(item: Connected) {
    if (item.builtin) drawer?.edit({ kind: 'builtin', id: item.builtin.id });
    else if (item.custom) drawer?.edit({ kind: 'custom', provider: item.custom });
  }

  function remove(item: Connected) {
    if (item.builtin) return removeBuiltin(item.builtin);
    if (item.custom) return removeCustom(item.custom);
  }

  const capsText = (p: { transcription: boolean; cleanup: boolean }) =>
    p.transcription && p.cleanup ? 'Transcription and clean-up' : p.transcription ? 'Transcription only' : 'Clean-up only';

  async function saveBuiltinKey(provider: BuiltinProvider['id'], raw: string): Promise<{ error?: string; notice?: string }> {
    const key = raw.trim();
    if (!key) return { error: 'Enter an API key.' };
    let validation: { status: 'valid' | 'invalid' | 'unknown'; message: string };
    try {
      const result = await invoke<{ status: 'valid' | 'invalid' | 'unknown'; message: string }>('validate_api_key', { provider, key });
      validation = { status: result.status, message: result.message };
    } catch (err) {
      if (handleConsentError(err)) return { error: 'Accept the Terms and Privacy Policy, then try again.' };
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
      await Promise.all([loadKeyStatus(), loadUsage()]);
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
{#snippet plus()}<svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true"><path d="M8 3v10M3 8h10" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" fill="none"/></svg>{/snippet}

<div class="page-head">
  <h2 class="settings-h page-title">Providers</h2>
  <GatedButton class="btn-primary btn-compact add" data-setting-target="custom-providers" onclick={() => drawer?.openPicker()}>
    {@render plus()}
    Add provider
  </GatedButton>
</div>
<p class="panel-note">A provider is an AI service that runs your models. {isAndroid ? 'Keys are encrypted in the Android Keystore.' : 'Keys stay on this device and are never shown again.'}</p>
<ConsentHint banner feature="Adding or changing cloud providers" thenEnable={() => drawer?.openPicker()} />

<div class="providers" class:loading={!loaded}>
  <div class="group-head">
    <h3 class="settings-subhead">Connected</h3>
    {#if count}<span class="group-meta">{count} {count === 1 ? 'provider' : 'providers'}</span>{/if}
  </div>
  {#if count}
    <ul class="list" aria-label="Your providers">
      {#each connected as item (item.id)}
        {@const use = providerUse(item.id, usage)}
        {@const level = item.needsKey ? 'idle' : usageLevel(use)}
        <li
          class="row"
          data-setting-target={`api-key-${item.id}`}
          animate:flip={{ duration: motionMs(MOTION_MS.base), easing: cubicOut }}
          in:fly={{ y: motionPx(-8), duration: motionMs(MOTION_MS.panel), easing: cubicOut }}
          out:listItemCollapse={{ duration: MOTION_MS.base }}
        >
          <div class="row-line">
            <ProviderTile name={item.name} logo={item.logo} plate={item.plate} color={item.color} />
            <div class="info">
              <span class="name">{item.name}</span>
              <span class="sub">
                {#if item.needsKey}
                  <span class="pill warn">Needs key</span>
                {:else}
                  <span class="use use-{level}"><i aria-hidden="true"></i><span>{usageLabel(use)}</span></span>
                {/if}
                {#if item.host}<span class="host">{item.host}</span>{/if}
              </span>
            </div>
            <div class="caps"><ProviderCaps transcription={item.transcription} cleanup={item.cleanup} /></div>
            <div class="actions">
              <GatedButton class="icon-btn" aria-label={item.builtin ? `Replace ${item.name} key` : `Edit ${item.name}`} title={item.builtin ? 'Replace key' : 'Edit'} disabled={busy} onclick={() => edit(item)}>{@render pencil()}</GatedButton>
              <button type="button" class="icon-btn danger" aria-label={`Remove ${item.name}`} title="Remove" aria-expanded={confirming === item.id} disabled={busy} onclick={() => (confirming = confirming === item.id ? null : item.id)}>{@render trash()}</button>
            </div>
          </div>
          {#if confirming === item.id}
            <div class="confirm" transition:slide={{ duration: motionMs(MOTION_MS.base), easing: cubicOut }}>
              <span>
                {item.builtin ? `Remove the saved ${item.name} key?` : `Remove ${item.name} and its saved key?`}
                {#if level !== 'idle'}{item.builtin ? `${item.name} models stop working until you add a key again.` : `Its models leave your ${level === 'main' ? 'selection' : 'backups'}.`}{/if}
              </span>
              <button class="btn-danger btn-compact" type="button" disabled={busy} onclick={() => remove(item)}>{item.builtin ? 'Remove' : 'Remove provider'}</button>
              <button class="btn-ghost btn-compact" type="button" disabled={busy} onclick={() => (confirming = null)}>Cancel</button>
            </div>
          {/if}
        </li>
      {/each}
    </ul>
  {:else if loaded}
    <div class="empty" in:fade={{ duration: motionMs(MOTION_MS.base) }}>
      <p class="empty-title">Nothing connected yet</p>
      <p class="empty-copy">Cloud models need a provider. On-device models work without one.</p>
      {#if !available.length}<GatedButton class="btn-primary btn-compact" onclick={() => drawer?.openPicker()}>Choose a provider</GatedButton>{/if}
    </div>
  {/if}

  {#if error}<div class="error"><Callout severity="error" compact>{error}</Callout></div>{/if}
  {#if notice}
    <p class="notice" role="status" transition:slide={{ duration: motionMs(MOTION_MS.base), easing: cubicOut }}>
      <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true"><path d="m3.5 8.5 3 3 6-7" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/></svg>
      <span>{notice}</span>
    </p>
  {/if}

  {#if loaded && (available.length || allowCustom)}
    <div class="group-head">
      <h3 class="settings-subhead">Available</h3>
      <span class="group-meta">Add a key to connect</span>
    </div>
    <div class="avail" role="group" aria-label="Available providers">
      {#each available as item (item.id)}
        <div
          class="avail-cell"
          animate:flip={{ duration: motionMs(MOTION_MS.base), easing: cubicOut }}
          in:fade={{ duration: motionMs(MOTION_MS.base) }}
          out:scale={{ start: 0.96, duration: motionMs(MOTION_MS.fast), easing: cubicOut }}
        >
          <GatedButton class="avail-tile" aria-label={`Add ${item.label}`} onclick={() => drawer?.edit({ kind: 'builtin', id: item.id })}>
            <ProviderTile name={item.label} logo={getProviderLogo(item.id)} plate={getProviderPlate(item.id)} />
            <span class="avail-text">
              <span class="avail-name">{item.label}</span>
              <span class="avail-caps">{capsText(item)}</span>
            </span>
            <span class="avail-plus" aria-hidden="true">{@render plus()}</span>
          </GatedButton>
        </div>
      {/each}
      {#if allowCustom}
        <div class="avail-cell wide">
          <GatedButton class="avail-tile more" aria-label="Browse other providers" onclick={() => drawer?.openPicker()}>
            <span class="more-mark" aria-hidden="true">
              <svg viewBox="0 0 16 16" width="16" height="16"><circle cx="3.5" cy="8" r="1.2" fill="currentColor"/><circle cx="8" cy="8" r="1.2" fill="currentColor"/><circle cx="12.5" cy="8" r="1.2" fill="currentColor"/></svg>
            </span>
            <span class="avail-text">
              <span class="avail-name">Other providers</span>
              <span class="avail-caps">Ollama, LM Studio, Anthropic, or your own endpoint</span>
            </span>
            <span class="avail-plus" aria-hidden="true">
              <svg viewBox="0 0 16 16" width="12" height="12"><path d="M6 3.5 10.5 8 6 12.5" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/></svg>
            </span>
          </GatedButton>
        </div>
      {/if}
    </div>
  {/if}

  {#if loaded}<p class="fineprint">Logos belong to their owners. Verenu is not affiliated with these companies.</p>{/if}
</div>

<ProviderDrawer
  bind:this={drawer}
  builtins={BUILTIN_PROVIDERS}
  {keyStatus}
  {allowCustom}
  {saveBuiltinKey}
  onchange={refresh}
  onnotice={flash}
/>

<style>
  .page-head { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: var(--settings-h-mb, 20px); }
  .page-head h2.page-title { margin: 0; }
  :global(.add) { display: inline-flex; align-items: center; gap: 6px; flex: none; }

  .providers { transition: opacity var(--ui-duration-base) var(--ui-ease-out); }
  .providers.loading { opacity: 0; }

  .group-head { display: flex; align-items: baseline; justify-content: space-between; gap: 12px; margin: 28px 0 8px; }
  .group-head:first-child { margin-top: 4px; }
  .group-head :global(.settings-subhead) { margin: 0; }
  .group-meta { font-size: 11.5px; color: var(--ink-mute); }

  .list { list-style: none; margin: 0; padding: 0; border: 1px solid var(--line); border-radius: var(--r-lg, 14px); background: var(--settings-card-bg, var(--bg-elev)); overflow: hidden; }
  .row { border-top: 1px solid var(--line); }
  .row:first-child { border-top: 0; }
  .row-line { display: grid; grid-template-columns: auto minmax(0, 1fr) auto auto; grid-template-areas: 'tile info caps actions'; align-items: center; column-gap: 12px; min-height: 64px; padding: 10px 10px 10px 14px; }
  .row-line > :global(.tile) { grid-area: tile; }
  .info { grid-area: info; min-width: 0; display: grid; gap: 3px; }
  .caps { grid-area: caps; display: flex; }
  .actions { grid-area: actions; display: flex; gap: 2px; }
  .name { font-size: 13.5px; font-weight: 600; color: var(--ink); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sub { display: flex; align-items: center; gap: 8px; min-width: 0; font-size: 11.5px; color: var(--ink-mute); }
  .use { display: inline-flex; align-items: center; gap: 6px; min-width: 0; }
  .use span { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .use i { flex: none; width: 7px; height: 7px; box-sizing: border-box; border-radius: 50%; border: 1.5px solid var(--ink-faint); transition: background-color var(--ui-duration-base) var(--ui-ease-out), border-color var(--ui-duration-base) var(--ui-ease-out); }
  .use-main { color: var(--ink-soft); }
  .use-main i { background: var(--success); border-color: var(--success); }
  .use-backup i { border-color: var(--success); }
  .host { flex: none; max-width: 45%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-family: var(--mono); font-size: 11px; color: var(--ink-faint); }
  .host::before { content: ''; display: inline-block; width: 1px; height: 9px; margin-right: 8px; background: var(--line-strong); }
  .pill { flex: none; font-size: 10.5px; font-weight: 600; padding: 2px 8px; border-radius: 999px; }
  .pill.warn { background: var(--warning-bg); color: var(--warning); }

  :global(.icon-btn) { width: 32px; height: 32px; display: grid; place-items: center; padding: 0; border: 0; border-radius: 9px; background: transparent; color: var(--ink-mute); cursor: pointer; transition: background-color var(--ui-duration-fast) var(--ui-ease-out), color var(--ui-duration-fast) var(--ui-ease-out), transform var(--ui-duration-fast) var(--ui-ease-out); }
  :global(.icon-btn):hover:not(:disabled) { background: var(--control-hover); color: var(--ink); }
  :global(.icon-btn).danger:hover:not(:disabled), :global(.icon-btn).danger[aria-expanded='true'] { background: var(--danger-bg); color: var(--danger); }
  :global(.icon-btn):active:not(:disabled) { transform: scale(0.92); }
  :global(.icon-btn):focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  :global(.icon-btn):disabled { opacity: 0.4; cursor: default; }

  .confirm { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; padding: 10px 14px 12px 62px; font-size: 12px; line-height: 1.45; color: var(--ink-soft); }
  .confirm span { flex: 1 1 200px; }

  .empty { display: grid; justify-items: start; gap: 4px; padding: 18px 16px; border: 1px dashed var(--line-strong); border-radius: var(--r-lg, 14px); }
  .empty-title { margin: 0; font-size: 13.5px; font-weight: 600; color: var(--ink); }
  .empty-copy { margin: 0; font-size: 12px; line-height: 1.45; color: var(--ink-mute); }
  .empty :global(.btn-primary) { margin-top: 8px; }

  .error { margin: 12px 0 0; }
  .notice { display: flex; align-items: center; gap: 8px; margin: 12px 0 0; padding: 9px 12px; border: 1px solid var(--success-line); border-radius: 10px; font-size: 12px; line-height: 1.4; background: var(--success-bg); color: var(--ink); }
  .notice svg { flex: none; color: var(--success); }

  .avail { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 8px; }
  .avail-cell { min-width: 0; }
  .avail-cell.wide { grid-column: 1 / -1; }
  :global(.avail-tile) { display: flex; align-items: center; gap: 12px; width: 100%; min-height: 60px; box-sizing: border-box; padding: 10px 12px; border: 1px solid var(--line); border-radius: var(--r-lg, 14px); background: transparent; color: var(--ink); font: inherit; text-align: left; cursor: pointer; transition: background-color var(--ui-duration-fast) var(--ui-ease-out), border-color var(--ui-duration-fast) var(--ui-ease-out), transform var(--ui-duration-fast) var(--ui-ease-out); }
  :global(.avail-tile):hover { background: var(--settings-card-bg, var(--bg-elev)); border-color: var(--line-strong); }
  :global(.avail-tile):active { transform: scale(0.985); }
  :global(.avail-tile.more) { border-style: dashed; border-color: var(--line-strong); }
  .avail-text { flex: 1; min-width: 0; display: grid; gap: 2px; }
  .avail-name { font-size: 13px; font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .avail-caps { font-size: 11.5px; line-height: 1.35; color: var(--ink-mute); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .avail-plus { flex: none; display: grid; place-items: center; width: 24px; height: 24px; border-radius: 50%; color: var(--ink-faint); transition: background-color var(--ui-duration-fast) var(--ui-ease-out), color var(--ui-duration-fast) var(--ui-ease-out), transform var(--ui-duration-base) var(--ui-ease-out); }
  :global(.avail-tile):hover .avail-plus { background: var(--ink); color: var(--bg-elev); transform: rotate(90deg); }
  :global(.avail-tile.more):hover .avail-plus { transform: translateX(2px); }
  .more-mark { flex: none; display: grid; place-items: center; width: 36px; height: 36px; border-radius: 10px; background: var(--paper-2); color: var(--ink-mute); }

  .fineprint { margin: 18px 0 0; font-size: 11px; color: var(--ink-faint); }

  @container settings-panel (max-width: 520px) {
    .row-line { grid-template-columns: auto minmax(0, 1fr) auto; grid-template-areas: 'tile info actions' '. caps caps'; row-gap: 8px; padding-block: 12px; }
    .confirm { padding-left: 14px; }
    .sub { flex-wrap: wrap; row-gap: 2px; }
    .use { align-items: baseline; }
    .use span { white-space: normal; }
    .host { max-width: 100%; }
    .host::before { display: none; }
    .avail { grid-template-columns: minmax(0, 1fr); }
    :global(.icon-btn) { width: 40px; height: 40px; }
  }

  @media (prefers-reduced-motion: reduce) {
    :global(.icon-btn), :global(.avail-tile), .avail-plus, .use i, .providers { transition: none; }
    :global(.avail-tile):hover .avail-plus { transform: none; }
  }
</style>
