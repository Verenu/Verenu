<script lang="ts">
  import { onMount } from 'svelte';
  import { fade, fly, slide } from 'svelte/transition';
  import { drawerSlide, modalBackdrop, MOTION_MS, motionMs, motionPx } from '../../motion';
  import { modalFocusTrap } from '../../modalFocus';
  import { portal } from '../../portal';
  import { presetLogoHtml } from '../../customProviderLogos';
  import { CUSTOM_PROVIDER_PRESETS, POPULAR_PRESET_IDS, PRESET_GROUPS, displayHost, monogram, presetById, presetForUrl, type CustomProviderPreset } from '../../customProviderPresets';
  import { invoke } from '../../tauri';
  import { saveSetting } from '../../settings';
  import { formatIpcError } from '../../errors';
  import { customProviderStore, refreshCustomProviders, type CustomProvider } from '../../customProviders.svelte';
  import CompactSelect from '../CompactSelect.svelte';
  import Toggle from '../Toggle.svelte';

  let { addRequest = 0 }: { addRequest?: number } = $props();
  let seenRequest: number | null = null;
  $effect(() => {
    if (seenRequest === null) { seenRequest = addRequest; return; }
    if (addRequest !== seenRequest) { seenRequest = addRequest; startPicking(); }
  });
  let drawerEl = $state<HTMLElement | null>(null);
  let picking = $state(false);
  let search = $state('');
  let preset = $state<CustomProviderPreset | null>(null);
  let editing = $state<CustomProvider | null>(null);
  let savedOriginal = $state<CustomProvider | null>(null);
  let keys = $state<Record<string, boolean>>({});
  let key = $state('');
  let transcriptionModels = $state('');
  let cleanupModels = $state('');
  let headers = $state('{}');
  let overrides = $state('{}');
  let error = $state('');
  let notice = $state('');
  let busy = $state(false);
  let deleting = $state<string | null>(null);
  const protocolLabels = { openai: 'OpenAI compatible', anthropic: 'Anthropic compatible', xai: 'xAI compatible' } as const;
  const protocols = [
    { value: 'openai', label: protocolLabels.openai },
    { value: 'anthropic', label: protocolLabels.anthropic },
    { value: 'xai', label: protocolLabels.xai },
  ];
  // A preset with two wire formats offers just those two, recommended first.
  const formatOptions = $derived(preset?.alt && !savedOriginal
    ? [{ value: preset.protocol, label: `${protocolLabels[preset.protocol]} (recommended)` }, { value: preset.alt.protocol, label: protocolLabels[preset.alt.protocol] }]
    : protocols);

  async function load() {
    await refreshCustomProviders();
    keys = await invoke<Record<string, boolean>>('get_api_key_status');
  }
  onMount(() => { load().catch(e => error = formatIpcError(e, 'Could not load custom providers')); });

  const drawerOpen = $derived(picking || !!editing);
  function closeDrawer() { picking = false; editing = null; preset = null; key = ''; error = ''; }
  // A stray click on the backdrop must not discard a half-filled form.
  function backdropClose() { if (picking && !busy) closeDrawer(); }
  function startPicking() {
    picking = true; search = ''; showMore = false; showAdvanced = false; editing = null; error = ''; notice = '';
  }
  function edit(provider?: CustomProvider, from?: CustomProviderPreset) {
    savedOriginal = provider ? structuredClone($state.snapshot(provider)) : null;
    preset = provider ? presetForUrl(provider.base_url, provider.protocol) ?? null : from ?? null;
    editing = provider ? structuredClone($state.snapshot(provider)) : {
      id: `custom:${crypto.randomUUID()}`, name: from && from.id !== 'blank' && !from.id.includes('compatible') ? from.name : '',
      protocol: from?.protocol ?? 'openai', base_url: from?.base_url ?? '',
      requires_key: from?.requires_key ?? true,
      supports_transcription: from?.supports_transcription ?? true, supports_cleanup: from?.supports_cleanup ?? true,
      auth_header: null, extra_headers: {}, body_overrides: null,
      transcription_models: from?.transcription_models.slice(0, 1) ?? [], cleanup_models: from?.cleanup_models.slice(0, 1) ?? [],
    };
    picking = false;
    key = '';
    transcriptionModels = editing.transcription_models.join('\n');
    cleanupModels = editing.cleanup_models.join('\n');
    headers = JSON.stringify(editing.extra_headers, null, 2);
    overrides = JSON.stringify(editing.body_overrides ?? {}, null, 2);
    error = ''; notice = ''; deleting = null;
  }
  function addModel(kind: 'transcription' | 'cleanup', model: string) {
    const current = models(kind === 'transcription' ? transcriptionModels : cleanupModels);
    const next = (current.includes(model) ? current.filter(m => m !== model) : [...current, model]).join('\n');
    if (kind === 'transcription') transcriptionModels = next; else cleanupModels = next;
  }
  const suggestedCleanup = $derived(
    preset?.alt && editing?.protocol === preset.alt.protocol ? preset.alt.cleanup_models : preset?.cleanup_models ?? []);
  const chosen = (text: string) => new Set(models(text));
  const popular = POPULAR_PRESET_IDS.map(id => presetById(id)).filter((p): p is CustomProviderPreset => !!p);
  const rest = (group: string) => CUSTOM_PROVIDER_PRESETS.filter(p => p.group === group && !POPULAR_PRESET_IDS.includes(p.id));
  const searchResults = $derived.by(() => {
    const q = search.trim().toLowerCase();
    return q ? CUSTOM_PROVIDER_PRESETS.filter(p => `${p.name} ${p.id} ${p.base_url} ${p.alt?.base_url ?? ''}`.toLowerCase().includes(q)) : [];
  });
  let showMore = $state(false);
  let showAdvanced = $state(false);
  const capabilityLabel = (p: { supports_transcription: boolean; supports_cleanup: boolean }) =>
    p.supports_transcription && p.supports_cleanup ? 'Transcription + cleanup' : p.supports_transcription ? 'Transcription' : 'Cleanup';
  const isLocalUrl = (url: string) => { try { const h = new URL(url).hostname; return h === 'localhost' || h === '127.0.0.1' || h === '[::1]' || /^(10\.|192\.168\.|172\.(1[6-9]|2\d|3[01])\.)/.test(h); } catch { return false; } };
  async function openExternal(url: string) {
    try { const { open } = await import('@tauri-apps/plugin-shell'); await open(url); }
    catch { window.open(url, '_blank', 'noopener'); }
  }
  function setProtocol(value: string) {
    if (!editing) return;
    const next = value as CustomProvider['protocol'];
    const format = preset?.alt && !savedOriginal
      ? next === preset.protocol ? { base_url: preset.base_url, cleanup_models: preset.cleanup_models } : next === preset.alt.protocol ? preset.alt : null
      : null;
    editing.protocol = next;
    if (format) {
      editing.base_url = format.base_url;
      cleanupModels = format.cleanup_models.slice(0, 1).join('\n');
    }
    if (next === 'anthropic') editing.supports_transcription = false;
    else if (preset?.supports_transcription && !savedOriginal) editing.supports_transcription = true;
  }
  const models = (text: string) => [...new Set(text.split('\n').map(x => x.trim()).filter(Boolean))];
  function object(text: string, label: string): Record<string, unknown> {
    const value = JSON.parse(text || '{}');
    if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error(`${label} must be a JSON object.`);
    return value;
  }
  async function save() {
    if (!editing || busy) return;
    busy = true; error = ''; notice = '';
    try {
      const extra = object(headers, 'Extra headers');
      if (Object.values(extra).some(v => typeof v !== 'string')) throw new Error('Extra header values must be strings.');
      const body = object(overrides, 'Request options');
      const provider: CustomProvider = {
        ...$state.snapshot(editing), name: editing.name.trim(), base_url: editing.base_url.trim().replace(/\/+$/, ''),
        auth_header: editing.auth_header?.trim() || null,
        extra_headers: extra as Record<string, string>, body_overrides: Object.keys(body).length ? body : null,
        transcription_models: editing.supports_transcription ? models(transcriptionModels) : [],
        cleanup_models: editing.supports_cleanup ? models(cleanupModels) : [],
      };
      if (!provider.name || !provider.base_url) throw new Error('Enter a name and base URL.');
      if (!provider.supports_transcription && !provider.supports_cleanup) throw new Error('Turn on transcription, cleanup, or both.');
      if (provider.supports_transcription && !provider.transcription_models.length) throw new Error('Add at least one transcription model ID.');
      if (provider.supports_cleanup && !provider.cleanup_models.length) throw new Error('Add at least one cleanup model ID.');
      const destinationChanged = savedOriginal && (savedOriginal.base_url !== provider.base_url || savedOriginal.protocol !== provider.protocol || savedOriginal.auth_header !== provider.auth_header);
      if (destinationChanged && keys[provider.id]) {
        if (provider.requires_key && !key.trim()) throw new Error('Enter the API key again for this endpoint. The previous key will be cleared.');
      }
      await refreshCustomProviders();
      const list = customProviderStore.providers.filter(p => p.id !== provider.id);
      await saveSetting('custom_providers', [...list, provider]);
      if (destinationChanged) keys[provider.id] = false;
      savedOriginal = provider;
      await refreshCustomProviders();
      if (key.trim()) {
        const validation = await invoke<{ status: string; message: string }>('validate_api_key', { provider: provider.id, key: key.trim() });
        if (validation.status === 'invalid') throw new Error(validation.message);
        await invoke('save_api_key', { provider: provider.id, key: key.trim() });
      }
      key = '';
      await load();
      closeDrawer();
      notice = 'Provider saved. Choose its models in Settings → Models.';
    } catch (e) {
      error = formatIpcError(e, 'Could not save this provider');
    } finally { busy = false; }
  }
  async function remove(provider: CustomProvider) {
    busy = true; error = '';
    try {
      await invoke('delete_custom_provider', { provider: provider.id });
      await load(); deleting = null;
      if (editing?.id === provider.id) editing = null;
      notice = 'Provider removed. Its selected models were removed from your fallback chains.';
    } catch (e) { error = formatIpcError(e, 'Could not remove this provider'); }
    finally { busy = false; }
  }
  async function clearKey(provider: CustomProvider) {
    busy = true; error = '';
    try { await invoke('delete_api_key', { provider: provider.id }); await load(); }
    catch (e) { error = formatIpcError(e, 'Could not remove this API key'); }
    finally { busy = false; }
  }
</script>

<svelte:window onkeydown={e => { if (e.key === 'Escape' && drawerOpen && !busy) { e.preventDefault(); e.stopPropagation(); closeDrawer(); } }} />

{#snippet card(p: CustomProviderPreset)}
  <button type="button" class="card" onclick={() => edit(undefined, p)}>
    {@render tile(p.id, p.name, p.color, p.mark)}
    <span class="card-body"><span class="card-name">{p.name}</span><span class="card-note">{p.group === 'advanced' ? p.note : p.supports_transcription && p.supports_cleanup ? 'Transcription + cleanup' : p.supports_transcription ? 'Transcription' : 'Cleanup'}</span></span>
  </button>
{/snippet}

{#snippet tile(id: string | undefined, name: string, color: string, mark?: string, big = false)}
  {@const logo = presetLogoHtml(id)}
  <span class="tile" class:big class:logo={!!logo} style:--tile={color} aria-hidden="true">{#if logo}{@html logo}{:else}{mark ?? monogram(name)}{/if}</span>
{/snippet}

<section class="custom-providers" class:empty={!customProviderStore.providers.length} aria-label="Custom providers">
  {#if customProviderStore.providers.length}
    <div class="section-heading"><h3>Custom providers</h3></div>
  {/if}

  {#each customProviderStore.providers as provider (provider.id)}
    {@const match = presetForUrl(provider.base_url, provider.protocol)}
    <div class="provider-row">
      {@render tile(match?.id, provider.name, match?.color ?? '#6B7280')}
      <div class="provider-info">
        <div class="label">{provider.name}
          <span class="pill" class:ok={keys[provider.id] || !provider.requires_key} class:warn={provider.requires_key && !keys[provider.id]}>{keys[provider.id] ? 'Key saved' : provider.requires_key ? 'Needs API key' : 'No key needed'}</span></div>
        <div class="desc endpoint">{displayHost(provider.base_url)}<span class="dot">·</span>{capabilityLabel(provider)}</div>
      </div>
      <div class="actions">
        <button class="btn-ghost btn-compact" onclick={() => edit(provider)} disabled={busy} aria-label="Edit {provider.name}">Edit</button>
        {#if keys[provider.id]}<button class="btn-ghost btn-compact" onclick={() => clearKey(provider)} disabled={busy}>Clear key</button>{/if}
        <button class="btn-ghost btn-compact" onclick={() => deleting = provider.id} disabled={busy}>Remove</button>
      </div>
      {#if deleting === provider.id}
        <div class="remove-confirm" transition:slide={{ duration: motionMs(MOTION_MS.fast) }}><p>Remove {provider.name}, its saved key, and its selected models?</p>
          <button class="btn-danger btn-compact" onclick={() => remove(provider)} disabled={busy}>Remove provider</button>
          <button class="btn-ghost btn-compact" onclick={() => deleting = null} disabled={busy}>Cancel</button></div>
      {/if}
    </div>
  {/each}

  {#if drawerOpen}
    <div class="drawer-wrap" use:portal>
      <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
      <div class="drawer-backdrop" role="presentation" onclick={backdropClose} in:modalBackdrop={{ duration: MOTION_MS.base }} out:modalBackdrop={{ duration: MOTION_MS.fast }}></div>
      <div class="drawer" role="dialog" aria-modal="true" aria-label={editing ? (savedOriginal ? 'Edit custom provider' : 'Set up custom provider') : 'Add custom provider'} tabindex="-1"
        bind:this={drawerEl}
        use:modalFocusTrap={{ active: true, initialFocus: () => drawerEl?.querySelector<HTMLElement>('.search, input') ?? drawerEl }}
        in:drawerSlide={{ duration: MOTION_MS.panel }} out:drawerSlide={{ duration: MOTION_MS.base }}>
        <header class="drawer-head">
          <span>{editing ? (savedOriginal ? 'Edit provider' : 'New provider') : 'Add custom provider'}</span>
          <button type="button" class="drawer-close" aria-label="Close" onclick={closeDrawer} disabled={busy}><svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"><path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" fill="none"/></svg></button>
        </header>
        <div class="drawer-body"><div class="views">
  {#if picking}
    <div class="view" in:fly={{ x: motionPx(28), duration: motionMs(MOTION_MS.base) }} out:fade={{ duration: motionMs(MOTION_MS.fast) }}>
    <div class="picker" role="group" aria-label="Choose a provider preset">
      <div class="picker-head"><h3>Choose a starting point</h3></div>
      <input class="search" type="search" bind:value={search} placeholder="Search providers, e.g. Ollama" aria-label="Search provider presets" />
      <p class="hint">Presets only fill in the form. These services are not tested or supported by Verenu, and you can change every field.</p>
      {#if search.trim()}
        <div class="group-title">{searchResults.length} result{searchResults.length === 1 ? '' : 's'}</div>
        <div class="grid">{#each searchResults as p (p.id)}{@render card(p)}{/each}</div>
        {#if !searchResults.length}
          <div class="no-match"><p>No preset matches “{search}”.</p><button type="button" class="btn-ghost btn-compact" onclick={() => edit(undefined, presetById('blank'))}>Start from scratch</button></div>
        {/if}
      {:else}
        <div class="group-title">Popular</div>
        <div class="grid">{#each popular as p (p.id)}{@render card(p)}{/each}</div>
        <div class="group-title">Local &amp; self-hosted<span>Nothing leaves your network</span></div>
        <div class="grid">{#each rest('local') as p (p.id)}{@render card(p)}{/each}</div>
        <button type="button" class="disclosure" aria-expanded={showMore} onclick={() => showMore = !showMore}><span class="chev" class:open={showMore}>›</span> More providers<span>{rest('cloud').length + rest('gateway').length} more</span></button>
        {#if showMore}
          <div class="disclosed" transition:slide={{ duration: motionMs(MOTION_MS.fast) }}>
            <div class="group-title">Providers</div>
            <div class="grid">{#each rest('cloud') as p (p.id)}{@render card(p)}{/each}</div>
            <div class="group-title">Gateways &amp; developer platforms<span>One key, many models</span></div>
            <div class="grid">{#each rest('gateway') as p (p.id)}{@render card(p)}{/each}</div>
          </div>
        {/if}
        <button type="button" class="disclosure" aria-expanded={showAdvanced} onclick={() => showAdvanced = !showAdvanced}><span class="chev" class:open={showAdvanced}>›</span> Advanced<span>Any compatible endpoint</span></button>
        {#if showAdvanced}
          <div class="disclosed" transition:slide={{ duration: motionMs(MOTION_MS.fast) }}>
            <div class="grid">{#each rest('advanced') as p (p.id)}{@render card(p)}{/each}</div>
          </div>
        {/if}
      {/if}
    </div>
    </div>
  {/if}

  {#if editing}
    <form class="provider-editor" onsubmit={e => { e.preventDefault(); void save(); }} aria-label="Custom provider editor" in:fly={{ x: motionPx(28), duration: motionMs(MOTION_MS.base) }} out:fade={{ duration: motionMs(MOTION_MS.fast) }}>
      <div class="editor-head">
        {@render tile(preset?.id, editing.name || preset?.name || '', preset?.color ?? '#6B7280', preset?.mark && !editing.name ? preset.mark : undefined, true)}
        <div><h3>{savedOriginal ? `Edit ${savedOriginal.name}` : preset && preset.id !== 'blank' ? `Set up ${preset.name}` : 'Add custom provider'}</h3>
          {#if preset && !savedOriginal && preset.note}<p class="hint">{preset.note}</p>{/if}</div>
        {#if !savedOriginal}<button type="button" class="btn-ghost btn-compact change" onclick={startPicking} disabled={busy}>Change preset</button>{/if}
      </div>

      <div class="form-grid">
        <label>Name<input bind:value={editing.name} maxlength="40" placeholder="My provider" required disabled={busy} /></label>
        <div class="field"><span>{preset?.alt && !savedOriginal ? 'API format' : 'Protocol'}</span><CompactSelect value={editing.protocol} options={formatOptions} label="Provider protocol" onchange={setProtocol} /></div>
        <label class="full">Base URL<input type="url" bind:value={editing.base_url} placeholder="https://api.example.com/v1" required disabled={busy} /><small>Include the API version prefix, such as /v1. Verenu adds the task's endpoint path.</small></label>
      </div>
      {#if editing.base_url.includes('YOUR_')}<p class="callout warn" role="note">Replace the placeholder in the base URL before saving.</p>{/if}
      <p class="callout" class:local={isLocalUrl(editing.base_url)}>
        {#if isLocalUrl(editing.base_url)}<span class="net-badge">Stays on your network</span>{/if}
        Dictated {editing.supports_transcription && editing.supports_cleanup ? 'audio and text' : editing.supports_transcription ? 'audio' : 'text'} will be sent to <strong>{displayHost(editing.base_url) || 'the endpoint you enter'}</strong>.
      </p>

      <div class="cap-grid" role="group" aria-label="What this provider handles">
        <div class="cap" class:on={editing.supports_transcription} class:off={editing.protocol === 'anthropic'}>
          <div><div class="cap-title">Transcription</div><div class="hint">{editing.protocol === 'anthropic' ? 'Not available for Anthropic-style endpoints' : 'Turn speech into text'}</div></div>
          <Toggle checked={editing.supports_transcription} disabled={editing.protocol === 'anthropic' || busy} label="Enable custom transcription" onchange={v => { if (editing) editing.supports_transcription = v; }} />
        </div>
        <div class="cap" class:on={editing.supports_cleanup}>
          <div><div class="cap-title">Cleanup</div><div class="hint">Polish text with a language model</div></div>
          <Toggle checked={editing.supports_cleanup} disabled={busy} label="Enable custom cleanup" onchange={v => { if (editing) editing.supports_cleanup = v; }} />
        </div>
      </div>

      <div class="form-grid">
        {#if editing.supports_transcription}
          <label>Transcription model IDs<textarea bind:value={transcriptionModels} placeholder="whisper-1" rows="3" disabled={busy}></textarea><small>One model ID per line.</small>
            {#if preset?.transcription_models.length}<span class="chips">{#each preset.transcription_models as m}<button type="button" class="chip" class:on={chosen(transcriptionModels).has(m)} aria-pressed={chosen(transcriptionModels).has(m)} onclick={() => addModel('transcription', m)} disabled={busy}>{m}</button>{/each}</span>{/if}</label>
        {/if}
        {#if editing.supports_cleanup}
          <label>Cleanup model IDs<textarea bind:value={cleanupModels} placeholder={editing.protocol === 'anthropic' ? 'claude-sonnet-4-5' : 'gpt-4o-mini'} rows="3" disabled={busy}></textarea><small>One model ID per line.</small>
            {#if suggestedCleanup.length}<span class="chips">{#each suggestedCleanup as m}<button type="button" class="chip" class:on={chosen(cleanupModels).has(m)} aria-pressed={chosen(cleanupModels).has(m)} onclick={() => addModel('cleanup', m)} disabled={busy}>{m}</button>{/each}</span>{/if}</label>
        {/if}
      </div>
      {#if preset && !savedOriginal && preset.id !== 'blank'}<p class="hint">Suggested model IDs come from the vendor's docs and may be out of date. Check their current model list.</p>{/if}

      <div class="cap" class:on={editing.requires_key}>
        <div><div class="cap-title">Requires API key</div><div class="hint">{editing.requires_key ? 'Sent with each request' : 'No authentication header will be sent'}</div></div>
        <Toggle checked={editing.requires_key} disabled={busy} label="Custom provider requires API key" onchange={v => { if (editing) editing.requires_key = v; }} />
      </div>
      {#if editing.requires_key}
        <label>API key<input type="password" bind:value={key} autocomplete="off" placeholder={keys[editing.id] ? 'Saved key stays unchanged unless you replace it' : preset?.key_hint ?? 'Enter your API key'} disabled={busy} />
          <small>Keys are optional while configuring. Add one before using a provider that requires it.
            {#if preset?.docs && !savedOriginal}<button type="button" class="link" onclick={() => openExternal(preset!.docs!)}>Get a key or read setup docs</button>{/if}</small></label>
      {/if}
      <details><summary>Advanced request settings</summary>
        <label>API key header<input bind:value={editing.auth_header} placeholder={editing.protocol === 'anthropic' ? 'x-api-key' : 'Authorization: Bearer (default)'} disabled={busy} /><small>Enter a header name to send the key as its raw value. Leave blank for the protocol default.</small></label>
        <label>Extra headers, JSON<textarea bind:value={headers} rows="3" spellcheck="false" disabled={busy}></textarea><small>Non-secret string values only. Put credentials in the API key field.</small></label>
        {#if editing.supports_cleanup}<label>Cleanup request options, JSON<textarea bind:value={overrides} rows="3" spellcheck="false" disabled={busy}></textarea><small>Extra fields such as temperature. Model, messages, system, token limit, and streaming are controlled by Verenu.</small></label>{/if}
      </details>
      {#if error}<p class="provider-error" role="alert">{error}</p>{/if}
      <div class="actions sticky"><button class="btn-primary" type="submit" disabled={busy}>{busy ? 'Saving…' : 'Save provider'}</button><button type="button" class="btn-ghost" onclick={closeDrawer} disabled={busy}>Cancel</button></div>
    </form>
  {/if}
        </div></div>
      </div>
    </div>
  {/if}
  {#if error && !editing}<p class="provider-error" role="alert">{error}</p>{/if}
  {#if notice}<p class="notice" role="status">{notice}</p>{/if}
</section>

<style>
  .drawer-wrap { position: fixed; inset: 0; z-index: 70; display: flex; justify-content: flex-end; }
  .drawer-backdrop { position: absolute; inset: 0; background: var(--overlay); }
  .drawer { position: relative; display: flex; flex-direction: column; width: min(520px, 100vw); height: 100%; box-sizing: border-box; background: var(--bg-elev); border-left: 1px solid var(--line); box-shadow: var(--shadow-elev); outline: none; }
  .drawer-head { flex: none; display: flex; align-items: center; justify-content: space-between; padding: 14px 18px; border-bottom: 1px solid var(--line-soft); font-size: 12px; font-weight: 600; color: var(--ink-soft); }
  .drawer-close { width: 28px; height: 28px; display: grid; place-items: center; padding: 0; border: 0; border-radius: 8px; background: transparent; color: var(--ink-soft); cursor: pointer; }
  .drawer-close:hover:not(:disabled) { background: var(--control-hover); color: var(--ink); }
  .drawer-body { flex: 1; min-height: 0; overflow-y: auto; padding: 18px; overscroll-behavior: contain; }
  .views { display: grid; }
  .views > :global(*) { grid-area: 1 / 1; min-width: 0; }
  .custom-providers { margin-top: 28px; border-top: 1px solid var(--line); padding-top: 22px; }
  .custom-providers.empty { margin: 0; border: 0; padding: 0; }
  .section-heading, .actions { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .section-heading { justify-content: space-between; align-items: flex-start; margin-bottom: 14px; }
  h3 { font-size: 15px; margin: 0; font-weight: 600; }
  .hint { margin: 0; font-size: 11px; line-height: 1.5; color: var(--ink-mute); }

  .tile { --tile: #6b7280; flex: none; width: 32px; height: 32px; border-radius: 9px; display: grid; place-items: center; background: var(--tile); color: #fff; font-size: 11px; font-weight: 700; letter-spacing: 0.02em; box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.14); }
  .tile.logo { background: #fff; color: #111; border: 1px solid var(--line); box-shadow: none; }
  :global([data-theme='dark']) .tile.logo { background: #fff; }
  .tile :global(svg) { width: 62%; height: 62%; }
  .tile.big { width: 40px; height: 40px; border-radius: 11px; font-size: 13px; }


  .provider-row { display: flex; align-items: center; gap: 12px; flex-wrap: wrap; padding: 12px 14px; margin-bottom: 8px; border: 1px solid var(--line); border-radius: 12px; background: var(--bg-elev); }
  .provider-info { flex: 1; min-width: 160px; }
  .provider-info .label { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; font-size: 13px; font-weight: 600; }
  .endpoint { overflow-wrap: anywhere; font-size: 11px; color: var(--ink-mute); margin-top: 2px; }
  .dot { margin: 0 6px; }
  .pill { font-size: 10px; font-weight: 600; padding: 2px 8px; border-radius: 999px; background: var(--paper-2); color: var(--ink-mute); }
  .pill.ok { background: var(--success-bg); color: var(--success); }
  .pill.warn { background: var(--warning-bg); color: var(--warning); }
  .remove-confirm { width: 100%; display: flex; align-items: center; gap: 8px; flex-wrap: wrap; padding-top: 10px; border-top: 1px solid var(--line-soft); }
  .remove-confirm p { flex-basis: 100%; margin: 0; font-size: 12px; }

  .picker { display: grid; gap: 10px; }
  .picker-head, .editor-head { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
  .search { font-family: inherit; }
  .group-title { display: flex; align-items: baseline; gap: 8px; flex-wrap: wrap; margin-top: 8px; font-size: 11px; font-weight: 700; letter-spacing: 0.06em; text-transform: uppercase; color: var(--ink-soft); }
  .group-title span { font-weight: 400; letter-spacing: 0; text-transform: none; color: var(--ink-mute); font-size: 11px; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(190px, 1fr)); gap: 8px; }
  .card { display: flex; align-items: center; gap: 10px; text-align: left; padding: 10px; border: 1px solid var(--line); border-radius: 10px; background: transparent; color: var(--ink); cursor: pointer; min-width: 0; font: inherit; transition: background 120ms ease, border-color 120ms ease, transform 120ms ease; }
  .card:hover { background: var(--control-hover); border-color: var(--line-strong); }
  .card:active { transform: scale(0.985); }
  .card:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .card-body { display: grid; min-width: 0; gap: 1px; }
  .card-name { font-size: 12px; font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .card-note { font-size: 10.5px; color: var(--ink-mute); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .disclosure { display: flex; align-items: center; gap: 8px; width: 100%; margin-top: 6px; padding: 10px 2px; border: 0; border-top: 1px solid var(--line-soft); background: none; color: var(--ink); font: inherit; font-size: 12px; font-weight: 600; cursor: pointer; text-align: left; }
  .disclosure span:last-child { margin-left: auto; font-weight: 400; color: var(--ink-mute); font-size: 11px; }
  .disclosure:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; border-radius: 6px; }
  .chev { display: inline-block; width: 10px; transition: transform 150ms ease; color: var(--ink-mute); }
  .chev.open { transform: rotate(90deg); }
  .disclosed { display: grid; gap: 10px; }
  .no-match { display: grid; justify-items: start; gap: 8px; font-size: 12px; color: var(--ink-soft); }
  .no-match p { margin: 0; }

  .provider-editor { display: grid; gap: 16px; }
  .editor-head { justify-content: flex-start; }
  .editor-head > div { flex: 1; min-width: 0; }
  .editor-head .change { margin-left: auto; }
  .form-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 14px; }
  .full { grid-column: 1 / -1; }
  label, .field { display: flex; flex-direction: column; gap: 7px; font-size: 12px; color: var(--ink-soft); min-width: 0; }
  input, textarea { width: 100%; box-sizing: border-box; padding: 9px 10px; border: 1px solid var(--line); border-radius: 8px; background: transparent; color: var(--ink); font-family: var(--mono); font-size: 12px; }
  textarea { resize: vertical; }
  input:focus, textarea:focus { outline: 2px solid var(--accent); outline-offset: 2px; }
  small { font-size: 11px; color: var(--ink-mute); line-height: 1.5; }

  .callout { margin: 0; padding: 9px 12px; border-radius: 10px; font-size: 11.5px; line-height: 1.5; background: var(--paper-2); color: var(--ink-soft); overflow-wrap: anywhere; }
  .callout.local { background: var(--success-bg); }
  .callout.warn { background: var(--warning-bg); color: var(--ink); }
  .callout strong { color: var(--ink); }
  .net-badge { display: inline-block; margin-right: 6px; padding: 1px 8px; border-radius: 999px; font-size: 10px; font-weight: 700; background: var(--success); color: #fff; }

  .cap-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 10px; }
  .cap { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 11px 13px; border: 1px solid var(--line); border-radius: 10px; transition: border-color 120ms ease, background 120ms ease; }
  .cap.on { border-color: var(--line-strong); background: var(--paper-2); }
  .cap.off { opacity: 0.6; }
  .cap-title { font-size: 12.5px; font-weight: 600; color: var(--ink); }

  .chips { display: flex; flex-wrap: wrap; gap: 6px; }
  .chip { max-width: 100%; padding: 3px 9px; border: 1px solid var(--line); border-radius: 999px; background: transparent; color: var(--ink-soft); font-family: var(--mono); font-size: 10.5px; cursor: pointer; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; transition: background 120ms ease, border-color 120ms ease; }
  .chip:hover:not(:disabled) { background: var(--control-hover); }
  .chip.on { background: var(--ink); border-color: var(--ink); color: var(--paper); }
  .chip:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .link { padding: 0; margin-left: 4px; border: 0; background: none; color: var(--ink); text-decoration: underline; cursor: pointer; font: inherit; }

  details { border-top: 1px solid var(--line); padding-top: 12px; }
  summary { cursor: pointer; font-size: 12px; color: var(--ink-soft); }
  details label { margin-top: 14px; }
  .actions.sticky { position: sticky; bottom: -18px; margin: 0 -18px -18px; padding: 12px 18px 16px; border-top: 1px solid var(--line-soft); background: linear-gradient(to top, var(--bg-elev) 70%, transparent); }
  .provider-error { color: var(--danger); font-size: 12px; margin: 0; }
  .notice { margin: 10px 0 0; padding: 9px 12px; border-radius: 10px; font-size: 12px; background: var(--success-bg); color: var(--ink); }
  @media (max-width: 560px) { .grid { grid-template-columns: repeat(2, minmax(0, 1fr)); } .card { padding: 8px; gap: 8px; } .card-name { white-space: normal; line-height: 1.25; } .form-grid, .cap-grid { grid-template-columns: 1fr; } .drawer-body { padding: 14px; } .actions.sticky { margin: 0 -14px -14px; bottom: -14px; padding: 12px 14px 16px; } }
  @container settings-panel (max-width: 520px) {
    .form-grid, .cap-grid { grid-template-columns: 1fr; }
    .grid { grid-template-columns: repeat(2, minmax(0, 1fr)); }
    .card { padding: 8px; gap: 8px; }
    .card-name { white-space: normal; line-height: 1.25; }
    .editor-head { flex-wrap: wrap; }
    .editor-head .change { margin-left: 0; }
  }
</style>
