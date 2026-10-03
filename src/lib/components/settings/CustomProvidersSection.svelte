<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '../../tauri';
  import { saveSetting } from '../../settings';
  import { formatIpcError } from '../../errors';
  import { customProviderStore, refreshCustomProviders, type CustomProvider } from '../../customProviders.svelte';
  import CompactSelect from '../CompactSelect.svelte';
  import Toggle from '../Toggle.svelte';

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
  const protocols = [
    { value: 'openai', label: 'OpenAI compatible' },
    { value: 'anthropic', label: 'Anthropic compatible' },
    { value: 'xai', label: 'xAI compatible' },
  ];

  async function load() {
    await refreshCustomProviders();
    keys = await invoke<Record<string, boolean>>('get_api_key_status');
  }
  onMount(() => { load().catch(e => error = formatIpcError(e, 'Could not load custom providers')); });

  function edit(provider?: CustomProvider) {
    savedOriginal = provider ? structuredClone($state.snapshot(provider)) : null;
    editing = provider ? structuredClone($state.snapshot(provider)) : {
      id: `custom:${crypto.randomUUID()}`, name: '', protocol: 'openai', base_url: '',
      requires_key: true, supports_transcription: true, supports_cleanup: true,
      auth_header: null, extra_headers: {}, body_overrides: null,
      transcription_models: [], cleanup_models: [],
    };
    key = '';
    transcriptionModels = editing.transcription_models.join('\n');
    cleanupModels = editing.cleanup_models.join('\n');
    headers = JSON.stringify(editing.extra_headers, null, 2);
    overrides = JSON.stringify(editing.body_overrides ?? {}, null, 2);
    error = ''; notice = ''; deleting = null;
  }
  function setProtocol(value: string) {
    if (!editing) return;
    editing.protocol = value as CustomProvider['protocol'];
    if (value === 'anthropic') editing.supports_transcription = false;
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
      editing = null;
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

<section class="custom-providers" aria-label="Custom providers" data-setting-target="custom-providers">
  <div class="section-heading"><h3>Custom providers</h3><button class="btn-ghost" onclick={() => edit()} disabled={busy || customProviderStore.providers.length >= 12}>Add custom provider</button></div>
  <p class="panel-note">Connect your own OpenAI, Anthropic, or xAI compatible endpoint. API keys stay in this device's credential store.</p>
  {#each customProviderStore.providers as provider (provider.id)}
    <div class="setting-row provider-row">
      <div class="provider-info"><div class="label">{provider.name}<span class="key-state">{keys[provider.id] ? 'Key saved' : provider.requires_key ? 'Needs API key' : 'No key required'}</span></div>
        <div class="desc endpoint">{provider.base_url}</div>
        <div class="desc">{[provider.supports_transcription ? 'Transcription' : '', provider.supports_cleanup ? 'Cleanup' : ''].filter(Boolean).join(' · ')}</div>
      </div>
      <div class="actions"><button class="btn-ghost btn-compact" onclick={() => edit(provider)} disabled={busy}>Edit</button>
        {#if keys[provider.id]}<button class="btn-ghost btn-compact" onclick={() => clearKey(provider)} disabled={busy}>Clear key</button>{/if}
        <button class="btn-ghost btn-compact" onclick={() => deleting = provider.id} disabled={busy}>Remove</button></div>
      {#if deleting === provider.id}<div class="remove-confirm"><p>Remove {provider.name}, its saved key, and its selected models?</p><button class="btn-danger btn-compact" onclick={() => remove(provider)} disabled={busy}>Remove provider</button><button class="btn-ghost btn-compact" onclick={() => deleting = null} disabled={busy}>Cancel</button></div>{/if}
    </div>
  {/each}
  {#if editing}
    <form class="provider-editor" onsubmit={e => { e.preventDefault(); void save(); }} aria-label="Custom provider editor">
      <h3>{savedOriginal ? 'Edit provider' : 'Add custom provider'}</h3>
      <div class="form-grid">
        <label>Name<input bind:value={editing.name} maxlength="40" placeholder="My provider" required disabled={busy} /></label>
        <div class="field"><span>Protocol</span><CompactSelect value={editing.protocol} options={protocols} label="Provider protocol" onchange={setProtocol} /></div>
        <label class="full">Base URL<input type="url" bind:value={editing.base_url} placeholder="https://api.example.com/v1" required disabled={busy} /><small>Include the API version prefix, such as /v1. Verenu adds the task's endpoint path.</small></label>
      </div>
      <p class="destination">Dictated {editing.supports_transcription && editing.supports_cleanup ? 'audio and text' : editing.supports_transcription ? 'audio' : 'text'} will be sent to <strong>{editing.base_url || 'the endpoint you enter'}</strong>.</p>
      <div class="capabilities"><span>Transcription</span><Toggle checked={editing.supports_transcription} disabled={editing.protocol === 'anthropic' || busy} label="Enable custom transcription" onchange={v => { if (editing) editing.supports_transcription = v; }} /><span>Cleanup</span><Toggle checked={editing.supports_cleanup} disabled={busy} label="Enable custom cleanup" onchange={v => { if (editing) editing.supports_cleanup = v; }} /></div>
      {#if editing.protocol === 'anthropic'}<p class="panel-note">Anthropic compatible endpoints support cleanup only.</p>{/if}
      <div class="form-grid">
        {#if editing.supports_transcription}<label>Transcription model IDs<textarea bind:value={transcriptionModels} placeholder="whisper-1" rows="3" disabled={busy}></textarea><small>One model ID per line.</small></label>{/if}
        {#if editing.supports_cleanup}<label>Cleanup model IDs<textarea bind:value={cleanupModels} placeholder={editing.protocol === 'anthropic' ? 'claude-sonnet-4-5' : 'gpt-4o-mini'} rows="3" disabled={busy}></textarea><small>One model ID per line.</small></label>{/if}
      </div>
      <div class="capabilities"><span>Requires API key</span><Toggle checked={editing.requires_key} disabled={busy} label="Custom provider requires API key" onchange={v => { if (editing) editing.requires_key = v; }} /></div>
      <label>API key<input type="password" bind:value={key} autocomplete="off" placeholder={keys[editing.id] ? 'Saved key stays unchanged unless you replace it' : 'Enter your API key'} disabled={busy} /><small>Keys are optional while configuring. Add one before using a provider that requires it.</small></label>
      <details><summary>Advanced request settings</summary>
        <label>API key header<input bind:value={editing.auth_header} placeholder={editing.protocol === 'anthropic' ? 'x-api-key' : 'Authorization: Bearer (default)'} disabled={busy} /><small>Enter a header name to send the key as its raw value. Leave blank for the protocol default.</small></label>
        <label>Extra headers, JSON<textarea bind:value={headers} rows="3" spellcheck="false" disabled={busy}></textarea><small>Non-secret string values only. Put credentials in the API key field.</small></label>
        {#if editing.supports_cleanup}<label>Cleanup request options, JSON<textarea bind:value={overrides} rows="3" spellcheck="false" disabled={busy}></textarea><small>Extra fields such as temperature. Model, messages, system, token limit, and streaming are controlled by Verenu.</small></label>{/if}
      </details>
      <div class="actions"><button class="btn-primary" type="submit" disabled={busy}>{busy ? 'Saving…' : 'Save provider'}</button><button type="button" class="btn-ghost" onclick={() => { editing = null; key = ''; error = ''; }} disabled={busy}>Cancel</button></div>
    </form>
  {/if}
  {#if error}<p class="provider-error" role="alert">{error}</p>{/if}
  {#if notice}<p class="panel-note" role="status">{notice}</p>{/if}
</section>

<style>
  .custom-providers { margin-top: 28px; border-top: 1px solid var(--line); padding-top: 22px; }
  .section-heading, .actions { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .section-heading { justify-content: space-between; }
  h3 { font-size: 15px; margin: 0; font-weight: 600; }
  .provider-row { gap: 12px; flex-wrap: wrap; }
  .provider-info { flex: 1; min-width: 180px; }
  .endpoint { overflow-wrap: anywhere; }
  .key-state { font-size: 10px; color: var(--ink-mute); margin-left: 10px; }
  .remove-confirm { width: 100%; display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .remove-confirm p { flex-basis: 100%; margin: 0; font-size: 12px; }
  .provider-editor { display: grid; gap: 16px; padding: 20px 0; }
  .form-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 14px; }
  .full { grid-column: 1 / -1; }
  label, .field { display: flex; flex-direction: column; gap: 7px; font-size: 12px; color: var(--ink-soft); min-width: 0; }
  input, textarea { width: 100%; box-sizing: border-box; padding: 9px 10px; border: 1px solid var(--line); border-radius: 6px; background: transparent; color: var(--ink); font-family: var(--mono); font-size: 12px; }
  textarea { resize: vertical; }
  input:focus, textarea:focus { outline: 2px solid var(--accent); outline-offset: 2px; }
  small, .destination { font-size: 11px; color: var(--ink-mute); line-height: 1.5; }
  .destination { margin: 0; overflow-wrap: anywhere; }
  .capabilities { display: flex; align-items: center; gap: 12px; flex-wrap: wrap; font-size: 12px; }
  details { border-top: 1px solid var(--line); padding-top: 12px; }
  summary { cursor: pointer; font-size: 12px; color: var(--ink-soft); }
  details label { margin-top: 14px; }
  .provider-error { color: var(--danger); font-size: 12px; }
  @container settings-panel (max-width: 520px) { .form-grid { grid-template-columns: 1fr; } }
</style>
