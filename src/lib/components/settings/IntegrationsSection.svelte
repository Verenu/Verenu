<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '../../tauri';
  import githubLight from '../../../assets/integrations/github-light.svg';
  import githubDark from '../../../assets/integrations/github-dark.svg';
  import { formatIpcError } from '../../errors';
  import { githubState, refreshGithub, setGithubUsername } from '../../githubStore.svelte';

  type Suggestion = { username: string; source: 'github_cli' | 'git_config' };
  let suggestion = $state<Suggestion | null>(null);
  let detecting = $state(true);
  let draft = $state('');
  let editing = $state(false);
  let saving = $state(false);
  let error = $state('');
  let alive = true;
  const connected = $derived(!!githubState.username);
  const showForm = $derived(!connected || editing);

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
    finally { if (alive) saving = false; }
  }
</script>

<h2 class="settings-h">Integrations</h2>
<p class="section-intro">Bring your coding activity into Insights.</p>

<section class="github-integration" aria-label="GitHub integration" data-setting-target="github-integration">
  <header class="service-head">
    <div class="service-icon" aria-hidden="true"><img class="github-light" src={githubLight} alt="" /><img class="github-dark" src={githubDark} alt="" /></div>
    <div class="service-title"><h3>GitHub</h3><p>Compare contributions such as commits, pull requests, and reviews with your dictated words.</p></div>
    {#if connected}<span class="connection-state" class:attention={!!githubState.error}>{githubState.loading ? 'Updating' : githubState.error ? 'Needs attention' : 'Connected'}</span>{/if}
  </header>

  {#if connected && !editing}
    <div class="connected-account"><span class="account-name">@{githubState.username}</span><span class="refresh-meta">Refreshes automatically every 15 minutes</span></div>
    <div class="actions">
      <button class="btn-ghost" disabled={saving} onclick={() => { draft = githubState.username; editing = true; error = ''; }}>Change account</button>
      <button class="btn-ghost" disabled={saving || githubState.loading} onclick={() => refreshGithub(true)}>Refresh now</button>
      <button class="btn-ghost" disabled={saving} onclick={disconnect}>Disconnect</button>
    </div>
  {/if}

  {#if showForm}
    <form onsubmit={connect}>
      <label for="github-username">GitHub username</label>
      <div class="connect-row">
        <div class="username-field" class:invalid={!!error}>
          <span aria-hidden="true">@</span>
          <input id="github-username" bind:value={draft} placeholder="Your username" maxlength="40" autocomplete="off" autocapitalize="none" spellcheck="false" disabled={saving} aria-invalid={!!error} aria-describedby="github-privacy github-form-status" oninput={() => { error = ''; }} />
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
      {#if editing}<button type="button" class="btn-ghost" disabled={saving} onclick={() => { editing = false; error = ''; }}>Cancel</button>{/if}
    </form>
  {/if}

  {#if error || githubState.error}<p class="connection-error" role="alert">{error || githubState.error}</p>{/if}
  {#if githubState.snapshot}
    <p class="refresh-meta" role="status">{githubState.loading ? 'Refreshing GitHub activity…' : `Last updated ${new Date(githubState.snapshot.fetched_at * 1000).toLocaleString()}`}</p>
    {#if githubState.snapshot.warning}<p class="connection-note" role="status">{githubState.snapshot.warning}</p>{/if}
  {/if}
  <p id="github-privacy" class="privacy-note">Verenu reads GitHub's public contribution calendar without signing in. It can include anonymized private counts only if Show private contributions is enabled on your profile. If the calendar cannot be read, Insights falls back to public commit search, which can miss activity and is limited to 1,000 results. Your dictated text stays on this device.</p>
</section>

<style>
  .section-intro { color: var(--ink-mute); font-size: 12.5px; line-height: 1.5; margin: -8px 0 24px; }
  .github-integration { border-top: 1px solid var(--line); border-bottom: 1px solid var(--line); padding: 24px 0; min-width: 0; }
  .service-head { display: flex; align-items: center; gap: 12px; margin-bottom: 24px; }
  .service-icon { display: grid; place-items: center; width: 40px; height: 40px; flex-shrink: 0; background: var(--control-hover); border: 1px solid var(--line); border-radius: 10px; color: var(--ink); }
  .service-icon img { width: 22px; height: 22px; }
  .github-dark { display: none; }
  :global(:root[data-theme="dark"]) .github-light { display: none; }
  :global(:root[data-theme="dark"]) .github-dark { display: block; }
  .service-title { min-width: 0; flex: 1; }
  h3 { margin: 0 0 4px; color: var(--ink); font-size: 15px; font-weight: 500; }
  .service-title p { margin: 0; color: var(--ink-mute); font-size: 12px; line-height: 1.5; }
  .connection-state { color: var(--success); font-size: 11px; white-space: nowrap; }
  .connection-state.attention { color: var(--ink-mute); }
  label { display: block; margin-bottom: 8px; color: var(--ink-soft); font-size: 12px; }
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
  .connected-account { display: flex; flex-direction: column; gap: 6px; margin-bottom: 16px; }
  .account-name { color: var(--ink); font-size: 14px; overflow-wrap: anywhere; }
  .actions { display: flex; flex-wrap: wrap; gap: 8px; margin-bottom: 12px; }
  .refresh-meta, .privacy-note, .connection-note, .connection-error { color: var(--ink-mute); font-size: 11.5px; line-height: 1.6; margin: 10px 0 0; }
  .privacy-note { margin-top: 20px; }
  .connection-error { color: var(--danger); }
  @container settings-panel (max-width: 440px) {
    .connect-row { flex-direction: column; }
    .connect-row > button { min-height: 38px; }
    .service-head { flex-wrap: wrap; }
    .connection-state { margin-left: 52px; }
  }
  @media (prefers-reduced-motion: reduce) { .username-field { transition: none; } }
</style>
