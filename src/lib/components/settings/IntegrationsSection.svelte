<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { fly } from 'svelte/transition';
  import T3Integration from './T3Integration.svelte';
  import IntegrationMenu from './IntegrationMenu.svelte';
  import IntegrationConfirmDialog from './IntegrationConfirmDialog.svelte';
  import { invoke } from '../../tauri';
  import githubLight from '../../../assets/integrations/github-light.svg';
  import githubDark from '../../../assets/integrations/github-dark.svg';
  import { formatIpcError } from '../../errors';
  import { icons } from '../../icons';
  import { motionMs, motionPx, MOTION_MS, MOTION_PX } from '../../motion';
  import { githubState, refreshGithub, setGithubUsername } from '../../githubStore.svelte';

  type Suggestion = { username: string; source: 'github_cli' | 'git_config' };
  let suggestion = $state<Suggestion | null>(null);
  let detecting = $state(true);
  let draft = $state('');
  let editing = $state(false);
  let saving = $state(false);
  let error = $state('');
  let showDetails = $state(false);
  let confirmDisconnect = $state<string | null>(null);
  let alive = true;
  const connected = $derived(!!githubState.username);
  const showForm = $derived(!connected || editing);
  const menuItems = $derived([
    { label: 'Refresh now', icon: icons.refresh, disabled: saving || githubState.loading, onselect: () => { void refreshGithub(true); } },
    { label: 'Change account', icon: icons.pencil, disabled: saving, onselect: changeAccount },
    { label: 'Disconnect', icon: icons.unlink, disabled: saving, onselect: () => { confirmDisconnect = githubState.username; } },
  ]);

  onMount(() => {
    alive = true;
    void refreshGithub().then(() => { if (alive && !draft) draft = githubState.username; });
    void invoke<Suggestion | null>('get_github_username_suggestion').then(value => {
      if (!alive) return;
      suggestion = value;
      if (!draft && !githubState.username && value) draft = value.username;
    }).catch(() => {}).finally(() => { if (alive) detecting = false; });
    return () => { alive = false; };
  });

  function changeAccount() {
    draft = githubState.username;
    editing = true;
    error = '';
    void tick().then(() => document.getElementById('github-username')?.focus());
  }

  function onWindowKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape' && showDetails && confirmDisconnect === null) showDetails = false;
  }

  async function connect(event: SubmitEvent) {
    event.preventDefault();
    const username = draft.trim().replace(/^@/, '');
    if (!/^[a-z0-9](?:[a-z0-9-]{0,37}[a-z0-9])?$/i.test(username) || username.includes('--')) {
      error = 'Use letters, numbers, and single hyphens for your GitHub username.';
      return;
    }
    saving = true;
    error = '';
    try {
      await setGithubUsername(username);
      if (alive) { draft = username; editing = !!githubState.error; }
    } catch (err) { if (alive) error = formatIpcError(err, 'Could not save GitHub username.'); }
    finally { if (alive) saving = false; }
  }

  async function disconnect() {
    saving = true;
    error = '';
    try {
      await setGithubUsername('');
      if (alive) { draft = ''; editing = false; }
    } catch (err) { if (alive) error = formatIpcError(err, 'Could not disconnect GitHub.'); }
    finally { if (alive) { saving = false; confirmDisconnect = null; } }
  }
</script>

<svelte:window onkeydown={onWindowKeydown} />

<h2 class="settings-h">Integrations</h2>
<p class="section-intro">Connect your tools for dictation and Insights.</p>

<T3Integration />

<section class="github-integration" aria-label="GitHub integration" data-setting-target="github-integration">
  <header class="integration-head">
    <div class="service-icon" aria-hidden="true"><img class="github-light" src={githubLight} alt="" /><img class="github-dark" src={githubDark} alt="" /></div>
    <div class="service-title">
      <h3>GitHub</h3>
      <p>Compare contributions such as commits, pull requests, and reviews with your dictated words.</p>
    </div>
    <div class="head-actions">
      <button
        type="button"
        class="icon-btn"
        aria-label="About GitHub"
        aria-expanded={showDetails}
        aria-controls={showDetails ? 'github-integration-details' : undefined}
        onclick={() => (showDetails = !showDetails)}
      >
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" aria-hidden="true">{@html icons.info}</svg>
      </button>
      {#if connected}
        <IntegrationMenu label="GitHub actions" triggerId="github-integration-menu" items={menuItems} />
      {/if}
    </div>
  </header>

  {#if connected && !editing}
    <div class="integration-body"><span class="account-name">@{githubState.username}</span></div>
  {/if}

  {#if githubState.loading}
    <p class="busy-line" role="status">Refreshing GitHub activity…</p>
  {:else if connected && !editing && githubState.snapshot?.warning}
    <p class="attention" role="status">Calendar unavailable. Showing public commit search, which can miss contributions.</p>
  {/if}

  {#if showForm}
    <form class="pairing-form" onsubmit={connect}>
      <label for="github-username">GitHub username</label>
      <div class="connect-row">
        <div class="username-field" class:invalid={!!error}>
          <span aria-hidden="true">@</span>
          <input id="github-username" bind:value={draft} placeholder="Your username" maxlength="40" autocomplete="off" autocapitalize="none" spellcheck="false" disabled={saving} aria-invalid={!!error} aria-describedby="github-form-status" oninput={() => { error = ''; }} />
        </div>
        <button type="submit" class="btn-primary" disabled={saving || !draft.trim()}>{saving ? 'Connecting…' : editing ? 'Save account' : 'Connect GitHub'}</button>
      </div>
      <div id="github-form-status" class="suggestion" role="status">
        {#if detecting}Checking for a signed-in account…
        {:else if suggestion}
          <span>Found in {suggestion.source === 'github_cli' ? 'GitHub CLI' : 'Git configuration'}.</span>
          {#if draft !== suggestion.username}<button type="button" class="btn-ghost btn-compact" onclick={() => { draft = suggestion!.username; error = ''; }}>Use @{suggestion.username}</button>{/if}
        {/if}
      </div>
      {#if editing}<button type="button" class="btn-ghost btn-compact cancel-edit" disabled={saving} onclick={() => { editing = false; error = ''; }}>Cancel</button>{/if}
    </form>
  {/if}

  {#if error || githubState.error}<p class="connection-error" role="alert">{error || githubState.error}</p>{/if}

  {#if showDetails}
    <dl id="github-integration-details" class="details" in:fly={{ y: -motionPx(MOTION_PX.nudge), duration: motionMs(MOTION_MS.fast) }}>
      <div><dt>Insights chart</dt><dd>Words and GitHub activity each use their own scale to compare trends, not equal counts. GitHub's profile calendar groups contributions such as commits, pull requests, and reviews by its calendar dates.</dd></div>
      {#if connected}
        <div><dt>Refresh</dt><dd>Refreshes automatically every 15 minutes.</dd></div>
      {/if}
      {#if githubState.snapshot}
        <div><dt>Last updated</dt><dd>{new Date(githubState.snapshot.fetched_at * 1000).toLocaleString()}</dd></div>
      {/if}
      {#if githubState.snapshot?.warning}
        <div><dt>Source</dt><dd>{githubState.snapshot.warning}</dd></div>
      {/if}
      <div><dt>Privacy</dt><dd id="github-privacy">Verenu reads GitHub's public contribution calendar without signing in. It can include anonymized private counts only if Show private contributions is enabled on your profile. If the calendar cannot be read, Insights falls back to public commit search, which can miss activity and is limited to 1,000 results. Your dictated text stays on this device.</dd></div>
    </dl>
  {/if}
</section>

{#if confirmDisconnect !== null}
  <IntegrationConfirmDialog
    title={`Disconnect @${confirmDisconnect}?`}
    body="Verenu removes this GitHub account and its cached contribution activity from this device. Insights stops showing GitHub contributions until you connect an account again."
    confirmLabel={saving ? 'Disconnecting…' : 'Disconnect'}
    busy={saving}
    oncancel={() => (confirmDisconnect = null)}
    onconfirm={disconnect}
    restoreFocus={() => document.getElementById('github-integration-menu') ?? document.getElementById('github-username')}
  />
{/if}

<style>
  .section-intro { color: var(--ink-mute); font-size: 12.5px; line-height: 1.5; margin: -8px 0 24px; }
  .github-integration { background: var(--bg-elev); border: 1px solid var(--line); border-radius: var(--r-md); margin-bottom: 14px; min-width: 0; padding: 18px 18px 16px; }
  .integration-head { align-items: flex-start; display: flex; gap: 12px; }
  .service-icon { display: grid; place-items: center; width: 44px; height: 44px; flex-shrink: 0; background: var(--control-hover); border: 1px solid var(--line); border-radius: 10px; color: var(--ink); }
  .service-icon img { width: 28px; height: 28px; }
  .github-dark { display: none; }
  :global(:root[data-theme="dark"]) .github-light { display: none; }
  :global(:root[data-theme="dark"]) .github-dark { display: block; }
  .service-title { flex: 1; min-width: 0; }
  h3 { color: var(--ink); font-size: 15px; font-weight: 500; margin: 0 0 3px; }
  .service-title p { color: var(--ink-mute); font-size: 12px; line-height: 1.5; margin: 0; }
  .head-actions { align-items: center; display: flex; flex-shrink: 0; gap: 2px; margin: -4px -6px 0 0; }
  .integration-body { display: flex; margin-top: 14px; min-width: 0; }
  .account-name { color: var(--ink); font-size: 14px; overflow-wrap: anywhere; }
  label { color: var(--ink-soft); display: block; font-size: 12px; margin: 0 0 8px; }
  .pairing-form { margin-top: 14px; }
  .connect-row { display: flex; align-items: stretch; gap: 10px; }
  .username-field { display: flex; align-items: center; gap: 8px; min-width: 0; flex: 1; background: var(--paper); border: 1px solid var(--line-strong); border-radius: var(--r-sm); padding: 0 12px; transition: border-color var(--ui-duration-fast) var(--ui-ease-out), box-shadow var(--ui-duration-fast) var(--ui-ease-out); }
  .username-field > span { color: var(--ink-faint); font-size: 14px; }
  .username-field:focus-within { border-color: var(--accent); box-shadow: var(--ui-focus-ring); }
  .username-field.invalid { border-color: var(--danger); }
  input { width: 100%; min-width: 0; height: 40px; padding: 0; border: 0; outline: none; background: transparent; color: var(--ink); font-family: var(--sans); font-size: 13px; }
  input::placeholder { color: var(--ink-faint); }
  input:disabled { opacity: var(--ui-disabled-opacity); }
  .suggestion { display: flex; align-items: center; flex-wrap: wrap; gap: 8px; margin-top: 8px; color: var(--ink-mute); font-size: 11.5px; line-height: 1.5; }
  .suggestion:empty { margin: 0; }
  .cancel-edit { margin-top: 10px; }
  .busy-line, .attention { align-items: flex-start; color: var(--ink-soft); display: flex; font-size: 12px; gap: 8px; line-height: 1.5; margin: 12px 0 0; }
  .attention::before { background: var(--warning); border-radius: 50%; content: ''; flex: 0 0 6px; height: 6px; margin-top: 6px; }
  .busy-line::before { animation: github-spin 0.9s linear infinite; border: 1.5px solid currentColor; border-radius: 50%; border-right-color: transparent; content: ''; flex: 0 0 8px; height: 8px; margin-top: 4px; }
  .details { border-top: 1px solid var(--line-soft); display: grid; gap: 10px; margin: 14px 0 0; padding: 14px 0 0; }
  .details > div { display: grid; gap: 12px; grid-template-columns: 84px minmax(0, 1fr); }
  .details dt { color: var(--ink-mute); font-size: 11.5px; }
  .details dd { color: var(--ink-soft); font-size: 12px; line-height: 1.6; margin: 0; overflow-wrap: anywhere; }
  .connection-error { color: var(--danger); font-size: 11.5px; line-height: 1.6; margin: 12px 0 0; overflow-wrap: anywhere; }
  @keyframes github-spin { to { transform: rotate(360deg); } }
  @container settings-panel (max-width: 440px) {
    .connect-row { flex-direction: column; }
    .connect-row > button { min-height: 38px; }
    .details > div { gap: 2px; grid-template-columns: 1fr; }
  }
  @media (prefers-reduced-motion: reduce) {
    .username-field { transition: none; }
    .busy-line::before { animation: none; }
  }
</style>
