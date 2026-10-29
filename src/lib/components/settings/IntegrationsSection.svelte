<script lang="ts">
  import { onMount, tick } from 'svelte';
  import T3Integration from './T3Integration.svelte';
  import IntegrationRow from './IntegrationRow.svelte';
  import IntegrationMenu from './IntegrationMenu.svelte';
  import IntegrationConfirmDialog from './IntegrationConfirmDialog.svelte';
  import { invoke } from '../../tauri';
  import githubLight from '../../../assets/integrations/github-light.svg';
  import githubDark from '../../../assets/integrations/github-dark.svg';
  import { formatIpcError } from '../../errors';
  import { icons } from '../../icons';
  import { githubState, refreshGithub, setGithubUsername } from '../../githubStore.svelte';
  import { timeAgo } from './t3SkillNames';

  type Suggestion = { username: string; source: 'github_cli' | 'git_config' };
  let suggestion = $state<Suggestion | null>(null);
  let detecting = $state(true);
  let draft = $state('');
  let editing = $state(false);
  let saving = $state(false);
  let error = $state('');
  let now = $state(Date.now());
  let confirmDisconnect = $state<string | null>(null);
  let alive = true;
  const connected = $derived(!!githubState.username);
  const showForm = $derived(!connected || editing);
  const menuItems = $derived([
    { label: 'Refresh now', icon: icons.refresh, disabled: saving || githubState.loading, onselect: () => { void refreshGithub(true); } },
    { label: 'Change account', icon: icons.pencil, disabled: saving, onselect: changeAccount },
    { label: 'Disconnect', icon: icons.unlink, danger: true, disabled: saving, onselect: () => { confirmDisconnect = githubState.username; } },
  ]);

  onMount(() => {
    alive = true;
    void refreshGithub().then(() => { if (alive && !draft) draft = githubState.username; });
    void invoke<Suggestion | null>('get_github_username_suggestion').then(value => {
      if (!alive) return;
      suggestion = value;
      if (!draft && !githubState.username && value) draft = value.username;
    }).catch(() => {}).finally(() => { if (alive) detecting = false; });
    const timer = setInterval(() => (now = Date.now()), 30_000);
    return () => { alive = false; clearInterval(timer); };
  });

  function changeAccount() {
    draft = githubState.username;
    editing = true;
    error = '';
    void tick().then(() => document.getElementById('github-username')?.focus());
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

<h2 class="settings-h">Integrations</h2>
<p class="section-intro">Connect your tools for dictation and Insights.</p>

<div class="integration-list">
  <T3Integration />

  <IntegrationRow name="GitHub" label="GitHub integration" target="github-integration" busy={githubState.loading}>
    {#snippet icon()}<img class="github-light" src={githubLight} alt="" /><img class="github-dark" src={githubDark} alt="" />{/snippet}
    {#snippet description()}Compare commits, pull requests, and reviews with your dictated words.{/snippet}
    {#snippet actions()}
      {#if connected}<IntegrationMenu label="GitHub actions" triggerId="github-integration-menu" items={menuItems} />{/if}
    {/snippet}

    {#if connected && !editing}
      <div class="int-status int-enter">
        <span class="int-state">
          {#if githubState.loading}<span class="int-spinner" aria-hidden="true"></span>Refreshing…
          {:else}<span class="int-dot" class:warn={!!githubState.snapshot?.warning} class:bad={!!githubState.error} aria-hidden="true"></span><span class="account-name">@{githubState.username}</span>{/if}
        </span>
        {#if githubState.snapshot}<span class="int-meta">Updated {timeAgo(githubState.snapshot.fetched_at, now)}</span>{/if}
      </div>
      {#if githubState.snapshot?.warning && !githubState.loading}<p class="int-note" role="status">Calendar unavailable. Showing public commit search, which can miss contributions.</p>{/if}
      <p class="int-fine">Reads GitHub's public contribution calendar without signing in. Your dictated text stays on this device.</p>
    {/if}

    {#if showForm}
      <form class="int-form int-enter" onsubmit={connect}>
        <label for="github-username">GitHub username</label>
        <div class="int-row">
          <div class="username-field" class:invalid={!!error}>
            <span aria-hidden="true">@</span>
            <input id="github-username" bind:value={draft} placeholder="Your username" maxlength="40" autocomplete="off" autocapitalize="none" spellcheck="false" disabled={saving} aria-invalid={!!error} aria-describedby="github-form-status" oninput={() => { error = ''; }} />
          </div>
          <button type="submit" class="btn-primary" disabled={saving || !draft.trim()}>{saving ? 'Connecting…' : editing ? 'Save account' : 'Connect GitHub'}</button>
          {#if editing}<button type="button" class="btn-ghost" disabled={saving} onclick={() => { editing = false; error = ''; }}>Cancel</button>{/if}
        </div>
        <div id="github-form-status" class="suggestion" role="status">
          {#if detecting}Checking for a signed-in account…
          {:else if suggestion}
            <span>Found in {suggestion.source === 'github_cli' ? 'GitHub CLI' : 'Git configuration'}.</span>
            {#if draft !== suggestion.username}<button type="button" class="btn-ghost btn-compact" onclick={() => { draft = suggestion!.username; error = ''; }}>Use @{suggestion.username}</button>{/if}
          {/if}
        </div>
      </form>
    {/if}

    {#if error || githubState.error}<p class="int-error" role="alert">{error || githubState.error}</p>{/if}
  </IntegrationRow>
</div>

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
  .section-intro { color: var(--ink-mute); font-size: 12.5px; line-height: 1.5; margin: -8px 0 22px; }
  .integration-list { border-bottom: 1px solid var(--line); }
  .github-dark { display: none; }
  :global(:root[data-theme="dark"]) .github-light { display: none; }
  :global(:root[data-theme="dark"]) .github-dark { display: block; }
  .account-name { overflow-wrap: anywhere; }
  .username-field { align-items: center; background: var(--paper); border: 1px solid var(--line); border-radius: var(--r-sm); display: flex; flex: 1; gap: 6px; min-height: 34px; min-width: 0; padding: 0 11px; transition: border-color var(--ui-duration-fast) var(--ui-ease-out); }
  .username-field > span { color: var(--ink-faint); font-size: 13px; }
  .username-field:focus-within { border-color: var(--line-strong); }
  .username-field.invalid { border-color: var(--danger); }
  input { background: transparent; border: 0; color: var(--ink); font-family: var(--sans); font-size: 13px; height: 32px; min-width: 0; outline: none; padding: 0; width: 100%; }
  input::placeholder { color: var(--ink-faint); }
  input:disabled { opacity: var(--ui-disabled-opacity); }
  .suggestion { align-items: center; color: var(--ink-mute); display: flex; flex-wrap: wrap; font-size: 11.5px; gap: 8px; line-height: 1.5; margin-top: 8px; }
  .suggestion:empty { margin: 0; }
  @media (prefers-reduced-motion: reduce) {
    .username-field { transition: none; }
  }
</style>
