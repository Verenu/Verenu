<script lang="ts">
  import { fade, fly, scale, slide } from 'svelte/transition';
  import { flip } from 'svelte/animate';
  import { cubicOut } from 'svelte/easing';
  import { drawerSlide, modalBackdrop, MOTION_MS, motionMs, motionPx } from '../../motion';
  import { modalFocusTrap } from '../../modalFocus';
  import { portal } from '../../portal';
  import { presetLogoHtml } from '../../customProviderLogos';
  import { CUSTOM_PROVIDER_PRESETS, POPULAR_PRESET_IDS, displayHost, presetById, presetForUrl, type CustomProviderPreset } from '../../customProviderPresets';
  import { invoke } from '../../tauri';
  import { handleConsentError } from '../../legal/consentStore.svelte';
  import { saveSetting } from '../../settings';
  import { formatIpcError } from '../../errors';
  import { getProviderLogo, getProviderPlate } from '../../setup/ProviderLogos';
  import { customProviderStore, refreshCustomProviders, type CustomProvider } from '../../customProviders.svelte';
  import Callout from '../Callout.svelte';
  import Disclosure from '../Disclosure.svelte';
  import CompactSelect from '../CompactSelect.svelte';
  import Toggle from '../Toggle.svelte';
  import ModelIdInput from './ModelIdInput.svelte';
  import ProviderCaps from './ProviderCaps.svelte';
  import ProviderTile from './ProviderTile.svelte';
  import type { BuiltinProvider } from './builtinProviders';

  type EditTarget = { kind: 'builtin'; id: BuiltinProvider['id'] } | { kind: 'custom'; provider: CustomProvider };

  let { builtins, keyStatus, allowCustom = true, saveBuiltinKey, onchange, onnotice }: {
    builtins: BuiltinProvider[];
    keyStatus: Record<string, boolean>;
    allowCustom?: boolean;
    saveBuiltinKey: (id: BuiltinProvider['id'], key: string) => Promise<{ error?: string; notice?: string }>;
    onchange: () => Promise<void>;
    onnotice: (message: string) => void;
  } = $props();

  let drawerEl = $state<HTMLElement | null>(null);
  let view = $state<'closed' | 'pick' | 'builtin' | 'custom'>('closed');
  let fromPicker = $state(false);
  let urlUpfront = $state(false);
  let search = $state('');
  let showMore = $state(false);
  let builtin = $state<BuiltinProvider | null>(null);
  let preset = $state<CustomProviderPreset | null>(null);
  let editing = $state<CustomProvider | null>(null);
  let savedOriginal = $state<CustomProvider | null>(null);
  let key = $state('');
  let transcriptionModels = $state<string[]>([]);
  let cleanupModels = $state<string[]>([]);
  let transcriptionDraft = $state('');
  let cleanupDraft = $state('');
  let headers = $state('{}');
  let overrides = $state('{}');
  let error = $state('');
  let busy = $state(false);

  const protocolLabels = { openai: 'OpenAI compatible', anthropic: 'Anthropic compatible', xai: 'xAI compatible' } as const;
  const protocols = [
    { value: 'openai', label: protocolLabels.openai },
    { value: 'anthropic', label: protocolLabels.anthropic },
    { value: 'xai', label: protocolLabels.xai },
  ];
  const formatOptions = $derived(preset?.alt && !savedOriginal
    ? [{ value: preset.protocol, label: `${protocolLabels[preset.protocol]} (recommended)` }, { value: preset.alt.protocol, label: protocolLabels[preset.alt.protocol] }]
    : protocols);

  export function openPicker() { startPicking(); }
  export function edit(target: EditTarget) {
    if (target.kind === 'builtin') openBuiltin(builtins.find(b => b.id === target.id)!, false);
    else editCustom(target.provider);
  }

  function autosize(node: HTMLTextAreaElement, _value: string) {
    const fit = () => { node.style.height = 'auto'; node.style.height = `${node.scrollHeight + 2}px`; };
    fit(); node.addEventListener('input', fit);
    return { update: () => requestAnimationFrame(fit), destroy: () => node.removeEventListener('input', fit) };
  }
  function focusOnMount(node: HTMLElement) { requestAnimationFrame(() => node.focus({ preventScroll: true })); }

  const drawerOpen = $derived(view !== 'closed');
  function closeDrawer() { view = 'closed'; editing = null; savedOriginal = null; builtin = null; preset = null; key = ''; error = ''; }
  function backdropClose() { if (!busy) closeDrawer(); }
  function goBack() { if (!busy) startPicking(); }
  function startPicking() { view = 'pick'; fromPicker = false; search = ''; showMore = false; editing = null; builtin = null; error = ''; }

  function openBuiltin(item: BuiltinProvider, picked: boolean) {
    builtin = item; key = ''; error = ''; fromPicker = picked; view = 'builtin';
  }
  function editCustom(provider?: CustomProvider, from?: CustomProviderPreset) {
    savedOriginal = provider ? structuredClone($state.snapshot(provider)) : null;
    preset = provider ? presetForUrl(provider.base_url, provider.protocol) ?? null : from ?? null;
    editing = provider ? structuredClone($state.snapshot(provider)) : {
      id: `custom:${crypto.randomUUID()}`, name: from && from.id !== 'blank' && !from.id.includes('compatible') ? from.name : '',
      protocol: from?.protocol ?? 'openai', base_url: from?.group === 'local' ? '' : from?.base_url ?? '',
      requires_key: from?.requires_key ?? true,
      supports_transcription: from?.supports_transcription ?? true, supports_cleanup: from?.supports_cleanup ?? true,
      auth_header: null, extra_headers: {}, body_overrides: null,
      transcription_models: from?.transcription_models.slice(0, 1) ?? [], cleanup_models: from?.cleanup_models.slice(0, 1) ?? [],
    };
    fromPicker = !provider;
    urlUpfront = !provider && !!from && from.group === 'local';
    key = '';
    transcriptionModels = [...editing.transcription_models];
    cleanupModels = [...editing.cleanup_models];
    transcriptionDraft = ''; cleanupDraft = '';
    headers = JSON.stringify(editing.extra_headers, null, 2);
    overrides = JSON.stringify(editing.body_overrides ?? {}, null, 2);
    error = '';
    view = 'custom';
  }

  const suggestedCleanup = $derived(
    preset?.alt && editing?.protocol === preset.alt.protocol ? preset.alt.cleanup_models : preset?.cleanup_models ?? []);
  const popular = POPULAR_PRESET_IDS.map(id => presetById(id)).filter((p): p is CustomProviderPreset => !!p);
  const rest = (group: string) => CUSTOM_PROVIDER_PRESETS.filter(p => p.group === group && !POPULAR_PRESET_IDS.includes(p.id));
  const available = $derived(builtins.filter(b => !keyStatus[b.id]));
  const searching = $derived(search.trim().length > 0);
  const matches = $derived.by(() => {
    const q = search.trim().toLowerCase();
    if (!q) return { builtins: [] as BuiltinProvider[], presets: [] as CustomProviderPreset[] };
    return {
      builtins: available.filter(b => `${b.label} ${b.id}`.toLowerCase().includes(q)),
      presets: allowCustom ? CUSTOM_PROVIDER_PRESETS.filter(p => p.id !== 'blank' && `${p.name} ${p.id} ${p.base_url} ${p.alt?.base_url ?? ''}`.toLowerCase().includes(q)) : [],
    };
  });
  const matchCount = $derived(matches.builtins.length + matches.presets.length);

  const showAll = $derived(!preset || preset.id === 'blank' || !!savedOriginal);
  const needsUrl = $derived(!showAll && urlUpfront);
  const urlExample = $derived(preset?.group === 'local' && !savedOriginal && preset.base_url ? preset.base_url.replace('localhost', '192.168.1.50') : 'https://api.example.com/v1');
  const isLocalUrl = (url: string) => { try { const h = new URL(url).hostname; return h === 'localhost' || h === '127.0.0.1' || h === '[::1]' || /^(10\.|192\.168\.|172\.(1[6-9]|2\d|3[01])\.)/.test(h); } catch { return false; } };
  async function openExternal(url: string) {
    try { const { open } = await import('@tauri-apps/plugin-shell'); await open(url); }
    catch { window.open(url, '_blank', 'noopener'); }
  }
  function setCapability(task: 'transcription' | 'cleanup', on: boolean) {
    if (!editing) return;
    const other = task === 'transcription' ? 'cleanup' : 'transcription';
    editing[`supports_${task}`] = on;
    if (!on && !editing[`supports_${other}`] && !(other === 'transcription' && editing.protocol === 'anthropic')) editing[`supports_${other}`] = true;
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
      cleanupModels = format.cleanup_models.slice(0, 1);
    }
    if (next === 'anthropic') { editing.supports_transcription = false; editing.supports_cleanup = true; }
    else if (preset?.supports_transcription && !savedOriginal) editing.supports_transcription = true;
  }
  const models = (list: string[]) => [...new Set(list.flatMap(x => x.split(/[\n,]/)).map(x => x.trim()).filter(Boolean))];
  function object(text: string, label: string): Record<string, unknown> {
    let value: unknown;
    try { value = JSON.parse(text || '{}'); } catch { throw new Error(`${label} isn't valid JSON.`); }
    if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error(`${label} must be a JSON object.`);
    return value as Record<string, unknown>;
  }

  async function clearSavedKey() {
    if (!savedOriginal || busy) return;
    busy = true; error = '';
    try {
      await invoke('delete_api_key', { provider: savedOriginal.id });
      await onchange();
    } catch (e) { error = formatIpcError(e, 'Could not remove this API key'); }
    finally { busy = false; }
  }

  async function saveBuiltin() {
    if (!builtin || busy) return;
    busy = true; error = '';
    try {
      const result = await saveBuiltinKey(builtin.id, key);
      if (result.error) { error = result.error; return; }
      await onchange();
      onnotice(`${builtin.label} added. ${result.notice ?? ''}`.trim());
      closeDrawer();
    } finally { busy = false; }
  }

  async function save() {
    if (!editing || busy) return;
    busy = true; error = '';
    try {
      const extra = object(headers, 'Extra headers');
      if (Object.values(extra).some(v => typeof v !== 'string')) throw new Error('Extra header values must be strings.');
      const body = object(overrides, 'Request options');
      const provider: CustomProvider = {
        ...$state.snapshot(editing), name: editing.name.trim(), base_url: editing.base_url.trim().replace(/\/+$/, ''),
        auth_header: editing.auth_header?.trim() || null,
        extra_headers: extra as Record<string, string>, body_overrides: Object.keys(body).length ? body : null,
        transcription_models: editing.supports_transcription ? models([...transcriptionModels, transcriptionDraft]) : [],
        cleanup_models: editing.supports_cleanup ? models([...cleanupModels, cleanupDraft]) : [],
      };
      if (!provider.name || !provider.base_url) throw new Error('Enter a name and base URL.');
      if (!provider.supports_transcription && !provider.supports_cleanup) throw new Error('Turn on transcription, cleanup, or both.');
      if (provider.supports_transcription && !provider.transcription_models.length) throw new Error('Add at least one transcription model ID.');
      if (provider.supports_cleanup && !provider.cleanup_models.length) throw new Error('Add at least one cleanup model ID.');
      for (const [label, list] of [['transcription', provider.transcription_models], ['cleanup', provider.cleanup_models]] as const) {
        if (list.length > 100) throw new Error(`Use at most 100 ${label} model IDs.`);
        if (list.some(m => m.length > 200)) throw new Error(`A ${label} model ID is longer than 200 characters.`);
      }
      const destinationChanged = savedOriginal && (savedOriginal.base_url !== provider.base_url || savedOriginal.protocol !== provider.protocol || savedOriginal.auth_header !== provider.auth_header);
      if (destinationChanged && keyStatus[provider.id]) {
        if (provider.requires_key && !key.trim()) throw new Error('Enter the API key again for this endpoint. The previous key will be cleared.');
      }
      await refreshCustomProviders();
      const list = customProviderStore.providers.filter(p => p.id !== provider.id);
      await saveSetting('custom_providers', [...list, provider]);
      savedOriginal = provider;
      await refreshCustomProviders();
      if (key.trim()) {
        try {
          const validation = await invoke<{ status: string; message: string }>('validate_api_key', { provider: provider.id, key: key.trim() });
          if (validation.status === 'invalid') throw new Error(validation.message);
          await invoke('save_api_key', { provider: provider.id, key: key.trim() });
        } catch (keyError) {
          if (handleConsentError(keyError)) { error = 'Accept the Terms and Privacy Policy, then save the key again.'; await onchange(); return; }
          error = `The provider was saved, but its API key was not. ${formatIpcError(keyError, 'The key could not be saved.')}`;
          await onchange();
          return;
        }
      }
      key = '';
      await onchange();
      closeDrawer();
      onnotice('Provider saved. Choose its models in Models.');
    } catch (e) {
      error = formatIpcError(e, 'Could not save this provider');
    } finally { busy = false; }
  }
</script>

<svelte:window onkeydown={e => { if (e.key === 'Escape' && drawerOpen && !busy && !document.querySelector('.ui-dropdown-trigger[aria-expanded="true"]')) { e.preventDefault(); e.stopPropagation(); closeDrawer(); } }} />

{#snippet builtinCard(b: BuiltinProvider)}
  <button type="button" class="card" onclick={() => openBuiltin(b, true)}>
    <ProviderTile name={b.label} logo={getProviderLogo(b.id)} plate={getProviderPlate(b.id)} />
    <span class="card-name">{b.label}</span>
    <ProviderCaps transcription={b.transcription} cleanup={b.cleanup} />
  </button>
{/snippet}

{#snippet card(p: CustomProviderPreset)}
  <button type="button" class="card" onclick={() => editCustom(undefined, p)}>
    <ProviderTile name={p.name} logo={presetLogoHtml(p.id)} color={p.color} mark={p.mark} />
    <span class="card-name">{p.name}</span>
    <ProviderCaps transcription={p.supports_transcription} cleanup={p.supports_cleanup} />
  </button>
{/snippet}

{#snippet keyField()}
  {#if editing?.requires_key}
    <label>
      <span class="label-row">API key
        {#if preset?.docs && !savedOriginal}<button type="button" class="link" onclick={() => openExternal(preset!.docs!)}>Get a key ↗</button>{/if}
      </span>
      <input type="password" bind:value={key} autocomplete="off" placeholder={keyStatus[editing.id] ? 'Saved. Enter a new key to replace it' : preset?.key_hint ?? 'Enter your API key'} disabled={busy} />
    </label>
  {/if}
{/snippet}

{#snippet clearKey()}
  {#if savedOriginal && keyStatus[savedOriginal.id]}
    <button type="button" class="link clear" disabled={busy} onclick={clearSavedKey}>Clear saved key</button>
  {/if}
{/snippet}

{#snippet destination()}
  {#if editing && editing.base_url.trim() && !isLocalUrl(editing.base_url)}
    <Callout severity="info" compact>{editing.supports_transcription && editing.supports_cleanup ? 'Audio and text' : editing.supports_transcription ? 'Audio' : 'Text'} will be sent to <strong>{displayHost(editing.base_url)}</strong>.</Callout>
  {/if}
{/snippet}

{#snippet advanced()}
  {#if editing}
    <Disclosure label="Advanced" id="provider-advanced" class="more-fields"><div class="fields">
      <label>API key header<input bind:value={editing.auth_header} placeholder={editing.protocol === 'anthropic' ? 'x-api-key' : 'Authorization: Bearer (default)'} disabled={busy} /></label>
      <label>Extra headers (JSON)<textarea use:autosize={headers} bind:value={headers} rows="3" spellcheck="false" disabled={busy}></textarea></label>
      {#if editing.supports_cleanup}<label>Cleanup request options (JSON)<textarea use:autosize={overrides} bind:value={overrides} rows="3" spellcheck="false" disabled={busy}></textarea></label>{/if}
    </div></Disclosure>
  {/if}
{/snippet}

{#snippet fullFields(urlHere: boolean)}
  {#if editing}
    <div class="form-grid">
      <label>Name<input bind:value={editing.name} maxlength="40" placeholder="My provider" required disabled={busy} /></label>
      <div class="field"><span>{preset?.alt && !savedOriginal ? 'API format' : 'Protocol'}</span><CompactSelect value={editing.protocol} options={formatOptions} label="Provider protocol" onchange={setProtocol} /></div>
      {#if urlHere}
        <label class="full">Base URL<input type="url" bind:value={editing.base_url} placeholder={urlExample} required disabled={busy} /></label>
      {/if}
    </div>
    {#if editing.base_url.includes('YOUR_')}<Callout severity="warning" compact role="note">Replace the placeholder in the base URL before saving.</Callout>{/if}

    <div class="cap-grid" role="group" aria-label="What this provider handles">
      <div class="cap" class:on={editing.supports_transcription} class:off={editing.protocol === 'anthropic'}>
        <ProviderCaps transcription iconOnly />
        <span class="cap-title">Transcription</span>
        <Toggle checked={editing.supports_transcription} disabled={editing.protocol === 'anthropic' || busy} label="Enable custom transcription" onchange={v => setCapability('transcription', v)} />
      </div>
      <div class="cap" class:on={editing.supports_cleanup}>
        <ProviderCaps cleanup iconOnly />
        <span class="cap-title">Clean-up</span>
        <Toggle checked={editing.supports_cleanup} disabled={editing.protocol === 'anthropic' || busy} label="Enable custom cleanup" onchange={v => setCapability('cleanup', v)} />
      </div>
    </div>

    <div class="models-row" class:no-tx={!editing.supports_transcription} class:no-cl={!editing.supports_cleanup}>
      <div class="mfield" class:off={!editing.supports_transcription} inert={!editing.supports_transcription} aria-hidden={!editing.supports_transcription}>
        <ModelIdInput id="tx-models" label="Transcription model IDs" bind:values={transcriptionModels} bind:draft={transcriptionDraft} placeholder="whisper-1" suggestions={preset?.transcription_models ?? []} disabled={busy} />
      </div>
      <div class="mfield" class:off={!editing.supports_cleanup} inert={!editing.supports_cleanup} aria-hidden={!editing.supports_cleanup}>
        <ModelIdInput id="cl-models" label="Cleanup model IDs" bind:values={cleanupModels} bind:draft={cleanupDraft} placeholder={editing.protocol === 'anthropic' ? 'claude-sonnet-4-5' : 'gpt-4o-mini'} suggestions={suggestedCleanup} disabled={busy} />
      </div>
    </div>

    <div class="cap" class:on={editing.requires_key}>
      <span class="cap-title">Requires API key</span>
      <Toggle checked={editing.requires_key} disabled={busy} label="Custom provider requires API key" onchange={v => { if (editing) editing.requires_key = v; }} />
    </div>
  {/if}
{/snippet}

{#if drawerOpen}
  <div class="drawer-wrap" use:portal>
    <div class="drawer-backdrop" role="presentation" onclick={backdropClose} in:modalBackdrop={{ duration: MOTION_MS.base }} out:modalBackdrop={{ duration: MOTION_MS.fast }}></div>
    <div class="drawer" role="dialog" aria-modal="true" aria-label={view === 'pick' ? 'Add provider' : savedOriginal || keyStatus[builtin?.id ?? ''] ? 'Edit provider' : 'Set up provider'} tabindex="-1"
      bind:this={drawerEl}
      use:modalFocusTrap={{ active: true, initialFocus: () => drawerEl?.querySelector<HTMLElement>('.search, input') ?? drawerEl }}
      in:drawerSlide={{ duration: MOTION_MS.panel }} out:drawerSlide={{ duration: MOTION_MS.base }}>
      <header class="drawer-head">
        <div class="head-left">
          {#if view !== 'pick' && fromPicker}
            <button type="button" class="drawer-close" aria-label="Back to providers" onclick={goBack} disabled={busy} transition:scale={{ start: 0.7, duration: motionMs(MOTION_MS.fast) }}><svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"><path d="M10 3L5 8l5 5" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" fill="none"/></svg></button>
          {/if}
          <span>{view === 'pick' ? 'Add provider' : view === 'builtin' ? (keyStatus[builtin?.id ?? ''] ? `Replace ${builtin?.label} key` : `Add ${builtin?.label}`) : savedOriginal ? `Edit ${savedOriginal.name}` : preset && preset.id !== 'blank' ? `Add ${preset.name}` : 'Add custom provider'}</span>
        </div>
        <button type="button" class="drawer-close" aria-label="Close" onclick={closeDrawer} disabled={busy}><svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"><path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" fill="none"/></svg></button>
      </header>
      <div class="drawer-body scroll-styled scroll-thumb-elev"><div class="views">
        {#if view === 'pick'}
          <div class="view" in:fly={{ x: motionPx(28), duration: motionMs(MOTION_MS.base) }} out:fade={{ duration: motionMs(MOTION_MS.fast) }}>
            <div class="picker" role="group" aria-label="Choose a provider">
              <div class="search-row">
                <input class="search" use:focusOnMount type="search" bind:value={search} placeholder="Search providers" aria-label="Search providers" />
                {#if allowCustom}
                  <button type="button" class="btn-ghost btn-compact own" onclick={() => editCustom(undefined, presetById('blank'))}><svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true"><path d="M8 3v10M3 8h10" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" fill="none"/></svg>Custom</button>
                {/if}
              </div>
              <div class="stack">
                {#if searching}
                  <div class="pane" in:fly={{ y: motionPx(8), duration: motionMs(MOTION_MS.base), easing: cubicOut }} out:fade={{ duration: motionMs(MOTION_MS.fast) }}>
                    {#if matchCount}<div class="grid">
                      {#each matches.builtins as b (b.id)}<div class="cell" animate:flip={{ duration: motionMs(MOTION_MS.base), easing: cubicOut }}>{@render builtinCard(b)}</div>{/each}
                      {#each matches.presets as p (p.id)}<div class="cell" animate:flip={{ duration: motionMs(MOTION_MS.base), easing: cubicOut }}>{@render card(p)}</div>{/each}
                    </div>{/if}
                    {#if !matchCount}
                      <div class="no-match" in:fade={{ duration: motionMs(MOTION_MS.base), delay: motionMs(60) }}>
                        <p>No match for “{search}”.</p>
                        {#if allowCustom}<button type="button" class="btn-ghost btn-compact" onclick={() => editCustom(undefined, presetById('blank'))}>Add a custom provider</button>{/if}
                      </div>
                    {/if}
                  </div>
                {:else}
                  <div class="pane" in:fly={{ y: motionPx(-6), duration: motionMs(MOTION_MS.base), easing: cubicOut }} out:fade={{ duration: motionMs(MOTION_MS.fast) }}>
                    {#if available.length}
                      <div class="group-title">Built in</div>
                      <div class="grid">{#each available as b (b.id)}{@render builtinCard(b)}{/each}</div>
                    {/if}
                    {#if allowCustom}
                      <div class="group-title">Popular</div>
                      <div class="grid">{#each popular as p (p.id)}{@render card(p)}{/each}</div>
                      <div class="group-title">On your own machine</div>
                      <div class="grid">{#each rest('local') as p (p.id)}{@render card(p)}{/each}</div>
                      <button type="button" class="disclosure" aria-expanded={showMore} onclick={() => showMore = !showMore}><svg class="ui-chevron" class:open={showMore} width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m6 9 6 6 6-6"/></svg> More providers<span>{rest('cloud').length + rest('gateway').length}</span></button>
                      {#if showMore}
                        <div class="disclosed" transition:slide={{ duration: motionMs(MOTION_MS.base), easing: cubicOut }}>
                          <div class="grid">{#each [...rest('cloud'), ...rest('gateway')] as p (p.id)}{@render card(p)}{/each}</div>
                        </div>
                      {/if}
                    {/if}
                  </div>
                {/if}
              </div>
            </div>
          </div>
        {/if}

        {#if view === 'builtin' && builtin}
          <form class="provider-editor" onsubmit={e => { e.preventDefault(); void saveBuiltin(); }} aria-label="Provider key form" in:fly={{ x: motionPx(28), duration: motionMs(MOTION_MS.base) }} out:fade={{ duration: motionMs(MOTION_MS.fast) }}>
            <div class="editor-head">
              <ProviderTile name={builtin.label} logo={getProviderLogo(builtin.id)} plate={getProviderPlate(builtin.id)} big />
              <h3>{builtin.label}</h3>
              <ProviderCaps transcription={builtin.transcription} cleanup={builtin.cleanup} />
            </div>
            <p class="editor-note">Paste a key from your {builtin.label} account. Verenu checks it with {builtin.label} before saving, then stores it in this device's secure storage.</p>
            <label>
              <span class="label-row">API key <button type="button" class="link" onclick={() => openExternal(builtin!.keyUrl)}>Get a key ↗</button></span>
              <input type="password" bind:value={key} use:focusOnMount autocomplete="off" aria-label={`${builtin.label} API key`} placeholder={keyStatus[builtin.id] ? 'Saved. Enter a new key to replace it' : builtin.placeholder} disabled={busy} aria-invalid={error ? 'true' : 'false'} />
            </label>
            <div class="actions sticky">
              {#if error}<div class="provider-error" transition:slide={{ duration: motionMs(MOTION_MS.fast) }}><Callout severity="error" compact>{error}</Callout></div>{/if}
              <button class="btn-primary" type="submit" disabled={busy || !key.trim()}>{busy ? 'Checking…' : keyStatus[builtin.id] ? 'Save key' : 'Add provider'}</button>
              <button type="button" class="btn-ghost" onclick={closeDrawer} disabled={busy}>Cancel</button>
            </div>
          </form>
        {/if}

        {#if view === 'custom' && editing}
          <form class="provider-editor" onsubmit={e => { e.preventDefault(); void save(); }} aria-label="Custom provider editor" in:fly={{ x: motionPx(28), duration: motionMs(MOTION_MS.base) }} out:fade={{ duration: motionMs(MOTION_MS.fast) }}>
            <div class="editor-head">
              <ProviderTile name={editing.name || preset?.name || ''} logo={presetLogoHtml(preset?.id)} color={preset?.color} mark={preset?.mark && !editing.name ? preset.mark : undefined} big />
              <h3>{editing.name || (preset && preset.id !== 'blank' ? preset.name : 'Custom provider')}</h3>
              <ProviderCaps transcription={editing.supports_transcription} cleanup={editing.supports_cleanup} />
            </div>
            {#if showAll}
              {@render fullFields(true)}
              {@render keyField()}
              {@render destination()}
              {@render advanced()}
            {:else}
              {@render keyField()}
              {#if needsUrl && editing}
                <label>Base URL<input type="url" bind:value={editing.base_url} placeholder={urlExample} required disabled={busy} /></label>
              {/if}
              {@render destination()}
              <Disclosure label="Customize" id="provider-customize" class="more-fields"><div class="fields">
                {@render fullFields(!needsUrl)}
                {@render advanced()}
              </div></Disclosure>
            {/if}
            {@render clearKey()}
            <div class="actions sticky">
              {#if error}<div class="provider-error" transition:slide={{ duration: motionMs(MOTION_MS.fast) }}><Callout severity="error" compact>{error}</Callout></div>{/if}
              <button class="btn-primary" type="submit" disabled={busy}>{busy ? 'Saving…' : savedOriginal ? 'Save provider' : 'Add provider'}</button>
              <button type="button" class="btn-ghost" onclick={savedOriginal ? closeDrawer : goBack} disabled={busy}>Cancel</button>
            </div>
          </form>
        {/if}
      </div></div>
    </div>
  </div>
{/if}

<style>
  .drawer-wrap { position: fixed; inset: 0; z-index: 70; display: flex; justify-content: flex-end; }
  .drawer-backdrop { position: absolute; inset: 0; background: var(--overlay); }
  .drawer { position: relative; display: flex; flex-direction: column; width: min(480px, 100vw); height: 100%; box-sizing: border-box; padding-top: var(--native-titlebar-height, 0px); background: var(--bg-elev); border-left: 1px solid var(--line); box-shadow: var(--shadow-elev); outline: none; }
  .head-left { display: flex; align-items: center; gap: 8px; min-width: 0; }
  .head-left span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .drawer-head { flex: none; display: flex; align-items: center; justify-content: space-between; gap: 8px; min-height: 56px; box-sizing: border-box; padding: 12px 14px 12px 20px; border-bottom: 1px solid var(--line-soft); font-size: 14px; font-weight: 600; letter-spacing: -0.01em; color: var(--ink); }
  .drawer-close { flex: none; width: 32px; height: 32px; display: grid; place-items: center; padding: 0; border: 0; border-radius: 9px; background: transparent; color: var(--ink-soft); cursor: pointer; transition: background-color var(--ui-duration-fast) var(--ui-ease-out), color var(--ui-duration-fast) var(--ui-ease-out), transform var(--ui-duration-fast) var(--ui-ease-out); }
  .head-left .drawer-close { margin-left: -8px; }
  .drawer-close:hover:not(:disabled) { background: var(--control-hover); color: var(--ink); }
  .drawer-close:active:not(:disabled) { transform: scale(0.92); }
  .drawer-close:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .drawer-body { flex: 1; min-height: 0; overflow-y: auto; padding: 20px; overscroll-behavior: contain; }
  .views { display: grid; min-height: 100%; }
  .views > :global(*) { grid-area: 1 / 1; min-width: 0; }
  h3 { font-size: 17px; margin: 0; font-weight: 600; letter-spacing: -0.015em; flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .actions { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }

  .picker { display: grid; gap: 14px; }
  .search-row { display: flex; gap: 8px; align-items: center; }
  .search { flex: 1; min-width: 0; font-family: inherit; font-size: 13px; }
  .search::-webkit-search-cancel-button { -webkit-appearance: none; appearance: none; }
  .own { display: inline-flex; align-items: center; gap: 6px; flex: none; }
  .stack { display: grid; }
  .stack > .pane { grid-area: 1 / 1; display: grid; gap: 8px; align-content: start; min-width: 0; }
  .cell { min-width: 0; }
  .group-title { margin: 10px 0 0 2px; font-size: 12px; font-weight: 600; color: var(--ink-mute); }
  .pane > .group-title:first-child { margin-top: 2px; }
  .grid { display: grid; grid-template-columns: 1fr; border: 1px solid var(--line); border-radius: var(--r-lg, 14px); overflow: hidden; }
  .grid:empty { display: none; }
  .grid > :global(* + *) { border-top: 1px solid var(--line-soft); }
  .card { display: flex; align-items: center; gap: 12px; width: 100%; min-height: 56px; box-sizing: border-box; text-align: left; padding: 9px 12px; border: 0; background: transparent; color: var(--ink); cursor: pointer; min-width: 0; font: inherit; transition: background-color var(--ui-duration-fast) var(--ui-ease-out); }
  .card:hover { background: var(--control-hover); }
  .card:active { background: var(--control-active); }
  .card:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; border-radius: 12px; }
  .card-name { flex: 1; min-width: 0; font-size: 13px; font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .disclosure { display: flex; align-items: center; gap: 8px; width: 100%; margin-top: 6px; padding: 10px 2px; border: 0; background: none; color: var(--ink-soft); font: inherit; font-size: 12px; font-weight: 600; cursor: pointer; text-align: left; transition: color var(--ui-duration-fast) var(--ui-ease-out); }
  .disclosure:hover { color: var(--ink); }
  .disclosure span:last-child { margin-left: auto; font-weight: 400; color: var(--ink-mute); font-size: 11.5px; font-variant-numeric: tabular-nums; }
  .disclosure:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; border-radius: 6px; }
  .disclosed { display: grid; gap: 10px; }
  .no-match { display: grid; justify-items: start; gap: 8px; padding: 6px 2px; font-size: 12px; color: var(--ink-soft); }
  .no-match p { margin: 0; overflow-wrap: anywhere; }

  .provider-editor { display: flex; flex-direction: column; gap: 16px; min-height: 100%; }
  .editor-head { display: flex; align-items: center; gap: 12px; }
  .editor-note { margin: -4px 0 0; font-size: 12.5px; line-height: 1.5; color: var(--ink-mute); }
  .models-row { display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 1fr); column-gap: 14px; transition: grid-template-columns 320ms cubic-bezier(0.22, 1, 0.36, 1), column-gap 320ms cubic-bezier(0.22, 1, 0.36, 1); }
  .models-row.no-tx { grid-template-columns: minmax(0, 0fr) minmax(0, 1fr); column-gap: 0; }
  .models-row.no-cl { grid-template-columns: minmax(0, 1fr) minmax(0, 0fr); column-gap: 0; }
  .mfield { min-width: 0; max-height: 900px; overflow: hidden; padding: 3px; margin: -3px; transition: opacity 220ms ease, max-height 320ms cubic-bezier(0.22, 1, 0.36, 1); }
  .mfield > :global(.mid) { min-width: 160px; }
  .mfield.off { opacity: 0; pointer-events: none; max-height: 0; }
  .form-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 14px; }
  .full { grid-column: 1 / -1; }
  label, .field { display: flex; flex-direction: column; gap: 7px; font-size: 12px; font-weight: 500; color: var(--ink-soft); min-width: 0; }
  .label-row { display: flex; align-items: baseline; justify-content: space-between; gap: 8px; }
  input, textarea { width: 100%; box-sizing: border-box; padding: 10px 12px; border: 1px solid var(--line-strong); border-radius: 10px; background: transparent; color: var(--ink); font-family: var(--mono); font-size: 12px; font-weight: 400; transition: border-color var(--ui-duration-fast) var(--ui-ease-out), box-shadow var(--ui-duration-fast) var(--ui-ease-out); }
  input::placeholder, textarea::placeholder { color: var(--ink-faint); }
  textarea { resize: none; overflow: hidden; }
  input:focus, textarea:focus { outline: none; border-color: var(--accent); box-shadow: var(--ui-focus-ring); }
  input[aria-invalid='true'] { border-color: var(--danger); }

  .cap-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 10px; }
  .cap { display: flex; align-items: center; gap: 10px; padding: 10px 12px; border: 1px solid var(--line); border-radius: 12px; transition: border-color var(--ui-duration-fast) var(--ui-ease-out), background-color var(--ui-duration-fast) var(--ui-ease-out); }
  .cap.on { border-color: var(--line-strong); background: var(--paper-2); }
  .cap.off { opacity: 0.6; }
  .cap-title { flex: 1; min-width: 0; font-size: 12.5px; font-weight: 600; color: var(--ink); }

  .link { padding: 0; border: 0; background: none; color: var(--ink-mute); text-decoration: underline; text-underline-offset: 2px; cursor: pointer; font: inherit; font-size: 11.5px; font-weight: 400; transition: color var(--ui-duration-fast) var(--ui-ease-out); }
  .link:hover { color: var(--ink); }
  .link.clear { align-self: flex-start; font-size: 12px; color: var(--danger); }

  .provider-editor :global(.more-fields) { border-top: 1px solid var(--line-soft); padding-top: 8px; }
  .fields { display: flex; flex-direction: column; gap: 14px; padding: 8px 3px 4px; }
  .actions.sticky { position: sticky; bottom: -20px; margin: auto -20px -20px; padding: 12px 20px 16px; border-top: 1px solid var(--line-soft); background: var(--bg-elev); }
  .provider-error { flex-basis: 100%; }
  @media (max-width: 560px) {
    .models-row, .models-row.no-tx, .models-row.no-cl { grid-template-columns: minmax(0, 1fr); column-gap: 0; row-gap: 14px; transition: none; }
    .mfield > :global(.mid) { min-width: 0; }
    .mfield.off { margin-top: -14px; } .form-grid, .cap-grid { grid-template-columns: 1fr; } .drawer-body { padding: 14px; } .actions.sticky { margin: auto -14px -14px; bottom: -14px; padding: 12px 14px 16px; } }
  @media (prefers-reduced-motion: reduce) { .card, .models-row, .mfield, .drawer-close, input, textarea { transition: none; } }
</style>
