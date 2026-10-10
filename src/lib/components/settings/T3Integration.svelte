<script lang="ts">
  import { onMount } from 'svelte';
  import { t3State, connectT3, updateT3 } from '../../t3Store.svelte';
  import { isAndroid } from '../../platform';
  import t3Icon from '../../../assets/integrations/t3-code.svg';

  let draft = $state('');
  let replacing = $state(false);
  let showSkills = $state(false);
  let query = $state('');
  const connection = $derived(t3State.status?.connection);
  const importedSkills = $derived(t3State.status?.skills ?? []);
  const expired = $derived(!!connection && connection.expiresAt * 1000 <= Date.now());
  const error = $derived(t3State.error || connection?.error || '');
  const usingSavedSkills = $derived(importedSkills.length > 0 && (expired || !!error));
  const skills = $derived(importedSkills.filter(skill => `${skill.name} ${skill.displayName ?? ''} ${skill.description ?? ''}`.toLowerCase().includes(query.toLowerCase())));
  const statusText = $derived(t3State.connecting ? 'Connecting' : t3State.loading ? 'Updating' : usingSavedSkills ? 'Using saved skills' : expired ? 'Reconnect' : error ? 'Needs attention' : importedSkills.length ? 'Connected' : 'No skills imported');

  onMount(() => { void updateT3(); });
  async function connect(event: SubmitEvent) {
    event.preventDefault();
    const link = draft;
    draft = '';
    if (await connectT3(link)) replacing = false;
  }
</script>

<section class="t3-integration" aria-label="T3 Code integration" aria-busy={t3State.loading || t3State.connecting} data-setting-target="t3-integration">
  <header>
    <div class="service-icon" aria-hidden="true"><img src={t3Icon} alt="" /></div>
    <div class="service-title"><h3>T3 Code</h3><p>Turn spoken skill requests into exact <code>$skill-name</code> mentions.</p></div>
    {#if connection}<span class="connection-state" class:attention={expired || !!error || (!importedSkills.length && !t3State.loading)}>{statusText}</span>{/if}
  </header>
  {#if isAndroid}
    <p>T3 skill pairing is available in Verenu on Windows, macOS, and Linux.</p>
  {:else if !connection || replacing}
    <form onsubmit={connect} aria-busy={t3State.connecting}>
      <label for="t3-pairing-link">T3 Code pairing link</label>
      <div class="connect-row">
        <input id="t3-pairing-link" class="ui-input" type="password" bind:value={draft} autocomplete="off" spellcheck="false" placeholder="Paste a fresh pairing link" disabled={t3State.connecting} aria-describedby="t3-pairing-help t3-privacy" />
        <button type="submit" class="btn-primary" disabled={t3State.connecting || !draft.trim()}>{t3State.connecting ? 'Connecting…' : 'Connect T3 Code'}</button>
      </div>
      <p id="t3-pairing-help">Create a pairing link in T3 Code’s Connections settings. Use localhost, a private LAN address, or an HTTPS address. Tailscale is optional. Skill mentions apply to dictation into the T3 desktop app.</p>
      {#if replacing}<button type="button" class="btn-ghost btn-compact" disabled={t3State.connecting} onclick={() => { replacing = false; draft = ''; }}>Cancel</button>{/if}
    </form>
  {/if}
  {#if t3State.notice}<p class="pairing-status" class:busy={t3State.connecting} role="status" aria-live="polite">{t3State.notice}</p>{/if}
  {#if connection && !replacing}
    <div class="connection-details"><strong>{connection.label}</strong><span>T3 Code {connection.version}</span></div>
    <p>Skills from all T3 providers and workspaces are combined automatically. Duplicate names appear once.</p>
    {#if !importedSkills.length}<p>T3 Code did not report any skills. Enable skills in T3 Code, then pull now.</p>{/if}
    <div class="actions">
      <button class="btn-ghost btn-compact" disabled={t3State.loading} onclick={() => updateT3('pull_t3_skills', { force: true })}>Pull now</button>
      <button class="btn-ghost btn-compact" disabled={!importedSkills.length} aria-expanded={showSkills} onclick={() => showSkills = !showSkills}>{showSkills ? 'Hide skills' : 'View skills'}</button>
      <button class="btn-ghost btn-compact" disabled={t3State.loading} onclick={() => { replacing = true; draft = ''; }}>Reconnect</button>
      <button class="btn-ghost btn-compact" disabled={t3State.loading} onclick={() => updateT3('disconnect_t3')}>Disconnect</button>
    </div>
    <p role="status">{importedSkills.length} shared skills available. Pulls automatically each day while Verenu is running. Last pulled {new Date(connection.fetchedAt * 1000).toLocaleString()}.</p>
    <p>{usingSavedSkills ? 'Using your last imported skills. Reconnect or pull again to update them.' : 'Saved skills remain available when T3 is offline, until a successful pull replaces them or you disconnect.'}</p>
    {#if showSkills && importedSkills.length}
      <label for="t3-skill-search">Search skills</label>
      <input id="t3-skill-search" class="ui-input" type="search" bind:value={query} placeholder="Name or description" />
      <ul class="skill-list scrollbar-standard" aria-label="Imported T3 skills">
        {#each skills as skill (skill.name)}<li><code>${skill.name}</code>{#if skill.description}<p>{skill.description}</p>{/if}</li>{/each}
      </ul>
      {#if !skills.length}<p>No matching skills.</p>{/if}
    {/if}
  {/if}
  {#if error && !t3State.connecting}<p class="error" role="alert">{error}</p>{/if}
  <p>Requires T3 Code {t3State.status?.minimumVersion ?? '0.46'} or newer.</p>
  <p id="t3-privacy">The pairing credential stays in your native credential store. Only skill names are sent to your selected cleanup provider for dictation in T3 Code. Descriptions stay local. Skill instructions are never imported. Cleanup must be enabled and its provider available.</p>
</section>

<style>
  .t3-integration { border-bottom: 1px solid var(--line); padding: 24px 0; min-width: 0; }
  header { display: flex; align-items: center; gap: 12px; margin-bottom: 20px; }
  .service-icon { display: grid; place-items: center; width: 44px; height: 44px; flex-shrink: 0; background: var(--control-hover); border: 1px solid var(--line); border-radius: 10px; }
  .service-icon img { width: 32px; height: 32px; }
  .service-title { min-width: 0; flex: 1; }
  h3 { margin: 0 0 4px; color: var(--ink); font-size: 15px; font-weight: 500; }
  p { color: var(--ink-mute); font-size: 11.5px; line-height: 1.6; margin: 10px 0; overflow-wrap: anywhere; }
  header p { font-size: 12px; margin: 0; }
  label { display: block; margin: 16px 0 8px; color: var(--ink-soft); font-size: 12px; }
  .connect-row { display: flex; align-items: stretch; gap: 10px; }
  .connect-row input { flex: 1; min-width: 0; }
  .connection-state { color: var(--success); font-size: 11px; white-space: nowrap; }
  .attention { color: var(--ink-mute); }
  .pairing-status { color: var(--ink-soft); }
  .pairing-status.busy::before { content: ''; display: inline-block; width: 8px; height: 8px; margin-right: 6px; border: 1.5px solid currentColor; border-right-color: transparent; border-radius: 50%; animation: t3-spin 0.9s linear infinite; }
  .connection-details { display: flex; flex-direction: column; gap: 5px; color: var(--ink-soft); font-size: 12px; overflow-wrap: anywhere; }
  .connection-details strong { font-weight: 500; color: var(--ink); font-size: 14px; }
  .actions { display: flex; flex-wrap: wrap; gap: 8px; margin: 16px 0; }
  .skill-list { list-style: none; padding: 0; margin: 12px 0; max-height: 320px; overflow-y: auto; }
  li { padding: 10px 0; border-bottom: 1px solid var(--line-soft); overflow-wrap: anywhere; }
  li p { margin: 4px 0 0; }
  code { color: var(--ink-soft); font-size: 11.5px; }
  .error { color: var(--danger); }
  @keyframes t3-spin { to { transform: rotate(360deg); } }
  @media (prefers-reduced-motion: reduce) { .pairing-status.busy::before { animation: none; } }
  @container settings-panel (max-width: 440px) { .connect-row { flex-direction: column; } .connect-row button { min-height: 40px; } header { flex-wrap: wrap; } .connection-state { margin-left: 56px; flex-basis: calc(100% - 56px); } }
</style>
