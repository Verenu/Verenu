<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { fly } from 'svelte/transition';
  import { t3State, connectT3, updateT3 } from '../../t3Store.svelte';
  import { isAndroid } from '../../platform';
  import { icons } from '../../icons';
  import { motionMs, motionPx, MOTION_MS, MOTION_PX } from '../../motion';
  import IntegrationMenu from './IntegrationMenu.svelte';
  import IntegrationConfirmDialog from './IntegrationConfirmDialog.svelte';
  import T3SkillsDialog from './T3SkillsDialog.svelte';
  import t3Icon from '../../../assets/integrations/t3-code.svg';

  let draft = $state('');
  let replacing = $state(false);
  let showSkills = $state(false);
  let showDetails = $state(false);
  let confirmDisconnect = $state<string | null>(null);
  let countButton = $state<HTMLButtonElement | null>(null);
  const connection = $derived(t3State.status?.connection);
  const importedSkills = $derived(t3State.status?.skills ?? []);
  const skillCount = $derived(importedSkills.length);
  const skillNoun = $derived(skillCount === 1 ? 'skill' : 'skills');
  const expired = $derived(!!connection && connection.expiresAt * 1000 <= Date.now());
  const error = $derived(t3State.error || connection?.error || '');
  const usingSavedSkills = $derived(skillCount > 0 && !!connection?.error);
  const attention = $derived.by(() => {
    if (!connection) return '';
    if (usingSavedSkills) {
      return expired
        ? 'Using saved skills. Pairing expired, so reconnect to update them.'
        : 'Using saved skills. The last pull failed, so pull again to update them.';
    }
    if (expired) return 'Pairing expired. Reconnect to pull skills again.';
    if (!skillCount && !t3State.loading) return 'T3 Code did not report any skills. Enable skills in T3 Code, then pull now.';
    return '';
  });
  const menuItems = $derived([
    { label: 'Pull now', icon: icons.refresh, disabled: t3State.loading, onselect: pullNow },
    { label: 'Reconnect', icon: icons.key, disabled: t3State.loading, onselect: startReconnect },
    { label: 'Disconnect', icon: icons.unlink, disabled: t3State.loading, onselect: openDisconnect },
  ]);

  onMount(() => { void updateT3(); });

  function pullNow() {
    void updateT3('pull_t3_skills', { force: true });
  }

  function startReconnect() {
    replacing = true;
    draft = '';
    void tick().then(() => document.getElementById('t3-pairing-link')?.focus());
  }

  function openDisconnect() {
    const count = skillCount;
    confirmDisconnect = `Verenu removes this pairing from your native credential store and deletes ${count ? `the ${count} imported skill ${count === 1 ? 'name' : 'names'}` : 'the imported skill list'}. Spoken skill mentions stop until you pair again.`;
  }

  async function disconnect() {
    await updateT3('disconnect_t3');
    confirmDisconnect = null;
  }

  async function connect(event: SubmitEvent) {
    event.preventDefault();
    const link = draft;
    draft = '';
    if (await connectT3(link)) replacing = false;
  }

  function onWindowKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape' && showDetails && !showSkills && confirmDisconnect === null) showDetails = false;
  }
</script>

<svelte:window onkeydown={onWindowKeydown} />

<section class="t3-integration" aria-label="T3 Code integration" aria-busy={t3State.loading || t3State.connecting} data-setting-target="t3-integration">
  <header class="integration-head">
    <div class="service-icon" aria-hidden="true"><img src={t3Icon} alt="" /></div>
    <div class="service-title">
      <h3>T3 Code</h3>
      <p>Turn spoken skill requests into exact <code>$skill-name</code> mentions.</p>
    </div>
    <div class="head-actions">
      <button
        type="button"
        class="icon-btn"
        aria-label="About T3 Code"
        aria-expanded={showDetails}
        aria-controls={showDetails ? 't3-integration-details' : undefined}
        onclick={() => (showDetails = !showDetails)}
      >
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" aria-hidden="true">{@html icons.info}</svg>
      </button>
      {#if connection}
        <IntegrationMenu label="T3 Code actions" triggerId="t3-integration-menu" items={menuItems} />
      {/if}
    </div>
  </header>

  {#if connection}
    <div class="integration-body">
      <button bind:this={countButton} type="button" class="skill-count ui-focus-ring" aria-haspopup="dialog" onclick={() => (showSkills = true)}>
        {skillCount} {skillNoun}
        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m9 6 6 6-6 6"/></svg>
      </button>
      <span class="connection-label">{connection.label}</span>
    </div>
    {#if t3State.loading && !t3State.connecting}
      <p class="busy-line" role="status">Updating T3 skills…</p>
    {:else if attention && !replacing}
      <p class="attention" role="status">{attention}</p>
    {/if}
  {/if}

  {#if isAndroid}
    <p class="android-note">T3 skill pairing is available in Verenu on Windows, macOS, and Linux.</p>
  {:else if !connection || replacing}
    <form class="pairing-form" onsubmit={connect} aria-busy={t3State.connecting}>
      {#if replacing}<p class="replace-note">Your current connection stays active until the new link connects.</p>{/if}
      <label for="t3-pairing-link">T3 Code pairing link</label>
      <div class="connect-row">
        <input id="t3-pairing-link" class="ui-input" type="password" bind:value={draft} autocomplete="off" spellcheck="false" placeholder="Paste a fresh pairing link" disabled={t3State.connecting} aria-describedby="t3-pairing-help" />
        <button type="submit" class="btn-primary" disabled={t3State.connecting || !draft.trim()}>{t3State.connecting ? 'Connecting…' : 'Connect T3 Code'}</button>
      </div>
      <p id="t3-pairing-help" class="help">Create a pairing link in T3 Code’s Connections settings. Use localhost, a private LAN address, or an HTTPS address. Tailscale is optional. Skill mentions apply to dictation into the T3 desktop app.</p>
      {#if replacing}<button type="button" class="btn-ghost btn-compact cancel-replace" disabled={t3State.connecting} onclick={() => { replacing = false; draft = ''; }}>Cancel</button>{/if}
    </form>
  {/if}

  {#if t3State.connecting}<p class="pairing-status" role="status">{t3State.notice || 'Connecting to T3 Code…'}</p>{/if}

  {#if showDetails}
    <dl id="t3-integration-details" class="details" in:fly={{ y: -motionPx(MOTION_PX.nudge), duration: motionMs(MOTION_MS.fast) }}>
      {#if connection}
        <div><dt>Version</dt><dd>T3 Code {connection.version}</dd></div>
      {/if}
      <div><dt>Requires</dt><dd>T3 Code {t3State.status?.minimumVersion ?? '0.46'} or newer.</dd></div>
      {#if connection}
        <div><dt>Skills</dt><dd>Skills from all T3 providers and workspaces are combined automatically. Duplicate names appear once.</dd></div>
        <div><dt>Refresh</dt><dd>Pulls automatically each day while Verenu is running. Last pulled {new Date(connection.fetchedAt * 1000).toLocaleString()}.</dd></div>
        <div><dt>Offline</dt><dd>Saved skills remain available when T3 is offline, until a successful pull replaces them or you disconnect.</dd></div>
      {/if}
      <div><dt>Privacy</dt><dd>The pairing credential stays in your native credential store. Only skill names are sent to your selected cleanup provider for dictation in T3 Code. Descriptions stay local. Skill instructions are never imported. Cleanup must be enabled and its provider available.</dd></div>
    </dl>
  {/if}

  {#if error && !t3State.connecting}<p class="error" role="alert">{error}</p>{/if}
</section>

{#if showSkills}
  <T3SkillsDialog
    skills={importedSkills}
    busy={t3State.loading}
    onpull={pullNow}
    onclose={() => (showSkills = false)}
    restoreFocus={() => countButton}
  />
{/if}

{#if confirmDisconnect !== null}
  <IntegrationConfirmDialog
    title="Disconnect T3 Code?"
    body={confirmDisconnect}
    confirmLabel={t3State.loading ? 'Disconnecting…' : 'Disconnect'}
    busy={t3State.loading}
    oncancel={() => (confirmDisconnect = null)}
    onconfirm={disconnect}
    restoreFocus={() => document.getElementById('t3-integration-menu') ?? document.getElementById('t3-pairing-link')}
  />
{/if}

<style>
  .t3-integration { background: var(--bg-elev); border: 1px solid var(--line); border-radius: var(--r-md); margin-bottom: 14px; min-width: 0; padding: 18px 18px 16px; }
  .integration-head { align-items: flex-start; display: flex; gap: 12px; }
  .service-icon { display: grid; place-items: center; width: 44px; height: 44px; flex-shrink: 0; background: var(--control-hover); border: 1px solid var(--line); border-radius: 10px; }
  .service-icon img { width: 32px; height: 32px; }
  .service-title { flex: 1; min-width: 0; }
  h3 { color: var(--ink); font-size: 15px; font-weight: 500; margin: 0 0 3px; }
  .service-title p { color: var(--ink-mute); font-size: 12px; line-height: 1.5; margin: 0; }
  .head-actions { align-items: center; display: flex; flex-shrink: 0; gap: 2px; margin: -4px -6px 0 0; }
  code { color: var(--ink-soft); font-size: 11.5px; }
  .integration-body { align-items: baseline; display: flex; flex-wrap: wrap; gap: 4px 14px; margin-top: 14px; }
  .skill-count { align-items: center; background: none; border: 0; border-radius: 4px; color: var(--ink); cursor: pointer; display: inline-flex; font-family: var(--sans); font-size: 14px; font-weight: 500; gap: 4px; padding: 0; }
  .skill-count:hover { color: var(--ink-strong); text-decoration: underline; text-decoration-thickness: 1px; text-underline-offset: 3px; }
  .skill-count svg { transition: transform var(--ui-duration-fast) var(--ui-ease-out); }
  .skill-count:hover svg { transform: translateX(2px); }
  .connection-label { color: var(--ink-mute); font-size: 12px; min-width: 0; overflow-wrap: anywhere; }
  label { color: var(--ink-soft); display: block; font-size: 12px; margin: 14px 0 8px; }
  .pairing-form { margin-top: 14px; }
  .connect-row { display: flex; align-items: stretch; gap: 10px; }
  .connect-row input { flex: 1; min-width: 0; }
  .help, .replace-note, .android-note { color: var(--ink-mute); font-size: 11.5px; line-height: 1.6; margin: 10px 0 0; }
  .replace-note { color: var(--ink-soft); margin: 0 0 4px; }
  .cancel-replace { margin-top: 10px; }
  .pairing-status, .busy-line, .attention { align-items: flex-start; color: var(--ink-soft); display: flex; font-size: 12px; gap: 8px; line-height: 1.5; margin: 12px 0 0; }
  .attention::before { background: var(--warning); border-radius: 50%; content: ''; flex: 0 0 6px; height: 6px; margin-top: 6px; }
  .pairing-status::before, .busy-line::before { animation: t3-spin 0.9s linear infinite; border: 1.5px solid currentColor; border-radius: 50%; border-right-color: transparent; content: ''; flex: 0 0 8px; height: 8px; margin-top: 4px; }
  .details { border-top: 1px solid var(--line-soft); display: grid; gap: 10px; margin: 14px 0 0; padding: 14px 0 0; }
  .details > div { display: grid; gap: 12px; grid-template-columns: 84px minmax(0, 1fr); }
  .details dt { color: var(--ink-mute); font-size: 11.5px; }
  .details dd { color: var(--ink-soft); font-size: 12px; line-height: 1.6; margin: 0; overflow-wrap: anywhere; }
  .error { color: var(--danger); font-size: 11.5px; line-height: 1.6; margin: 12px 0 0; overflow-wrap: anywhere; }
  @keyframes t3-spin { to { transform: rotate(360deg); } }
  @media (prefers-reduced-motion: reduce) {
    .pairing-status::before, .busy-line::before { animation: none; }
    .skill-count svg { transition: none; }
  }
  @container settings-panel (max-width: 440px) {
    .connect-row { flex-direction: column; }
    .connect-row button { min-height: 40px; }
    .details > div { gap: 2px; grid-template-columns: 1fr; }
  }
</style>
