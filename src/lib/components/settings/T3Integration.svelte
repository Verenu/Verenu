<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { t3State, connectT3, updateT3 } from '../../t3Store.svelte';
  import { isAndroid } from '../../platform';
  import { icons } from '../../icons';
  import IntegrationRow from './IntegrationRow.svelte';
  import IntegrationMenu from './IntegrationMenu.svelte';
  import IntegrationConfirmDialog from './IntegrationConfirmDialog.svelte';
  import T3SkillsDialog from './T3SkillsDialog.svelte';
  import { isSkillExcluded, sortedSkillNames, timeAgo } from './t3SkillNames';
  import t3Icon from '../../../assets/integrations/t3-code.svg';

  let draft = $state('');
  let replacing = $state(false);
  let showSkills = $state(false);
  let confirmDisconnect = $state<string | null>(null);
  let countButton = $state<HTMLButtonElement | null>(null);
  let now = $state(Date.now());
  const connection = $derived(t3State.status?.connection);
  const importedSkills = $derived(t3State.status?.skills ?? []);
  const skillNames = $derived(sortedSkillNames(importedSkills));
  const skillCount = $derived(skillNames.length);
  const activeCount = $derived(skillNames.filter((name) => !isSkillExcluded(connection?.excludedSkills, name)).length);
  const skillNoun = $derived(skillCount === 1 ? 'skill' : 'skills');
  const countText = $derived(activeCount === skillCount ? `${skillCount} ${skillNoun}` : `${activeCount} of ${skillCount} ${skillNoun}`);
  const expired = $derived(!!connection && connection.expiresAt * 1000 <= now);
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
  const updating = $derived(t3State.loading && !t3State.connecting);
  const menuItems = $derived([
    { label: 'Pull now', icon: icons.refresh, disabled: t3State.loading, onselect: pullNow },
    { label: 'Reconnect', icon: icons.key, disabled: t3State.loading, onselect: startReconnect },
    { label: 'Disconnect', icon: icons.unlink, danger: true, disabled: t3State.loading, onselect: openDisconnect },
  ]);

  onMount(() => {
    void updateT3();
    const timer = setInterval(() => (now = Date.now()), 30_000);
    return () => clearInterval(timer);
  });

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
</script>

<IntegrationRow name="T3 Code" label="T3 Code integration" target="t3-integration" busy={t3State.loading || t3State.connecting}>
  {#snippet icon()}<img src={t3Icon} alt="" />{/snippet}
  {#snippet description()}Turn spoken skill requests into exact <code>$skill-name</code> mentions.{/snippet}
  {#snippet actions()}
    {#if connection}
      <IntegrationMenu label="T3 Code actions" triggerId="t3-integration-menu" items={menuItems} />
    {/if}
  {/snippet}

  {#if connection && !replacing}
    <div class="int-status int-enter">
      <span class="int-state">
        {#if updating}<span class="int-spinner" aria-hidden="true"></span>Updating…
        {:else}<span class="int-dot" class:warn={!!attention} aria-hidden="true"></span>{connection.label}{/if}
      </span>
      <button bind:this={countButton} type="button" class="int-link ui-focus-ring" aria-haspopup="dialog" onclick={() => (showSkills = true)}>
        {countText}
        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m9 6 6 6-6 6"/></svg>
      </button>
      <span class="int-meta">Pulled {timeAgo(connection.fetchedAt, now)}</span>
    </div>
    {#if attention}<p class="int-note" role="status">{attention}</p>{/if}
    <p class="int-fine">Only skill names reach your cleanup provider. Your pairing stays in your credential store.</p>
  {/if}

  {#if isAndroid}
    <p class="int-help">T3 skill pairing is available in Verenu on Windows, macOS, and Linux.</p>
  {:else if !connection || replacing}
    <form class="int-form int-enter" onsubmit={connect} aria-busy={t3State.connecting}>
      <label for="t3-pairing-link">{replacing ? 'New pairing link' : 'T3 Code pairing link'}</label>
      <div class="int-row">
        <input id="t3-pairing-link" class="ui-input" type="password" bind:value={draft} autocomplete="off" spellcheck="false" placeholder="Paste a fresh pairing link" disabled={t3State.connecting} aria-describedby="t3-pairing-help" />
        <button type="submit" class="btn-primary" disabled={t3State.connecting || !draft.trim()}>{t3State.connecting ? 'Connecting…' : 'Connect T3 Code'}</button>
        {#if replacing}<button type="button" class="btn-ghost" disabled={t3State.connecting} onclick={() => { replacing = false; draft = ''; }}>Cancel</button>{/if}
      </div>
      <p id="t3-pairing-help" class="int-help">Create a link in T3 Code under Settings, Connections. Use localhost, a private LAN address, or an HTTPS address. Needs T3 Code {t3State.status?.minimumVersion ?? '0.46'} or newer.</p>
    </form>
  {/if}

  {#if t3State.connecting}<p class="int-state int-status-line" role="status"><span class="int-spinner" aria-hidden="true"></span>{t3State.notice || 'Connecting to T3 Code…'}</p>{/if}
  {#if error && !t3State.connecting}<p class="int-error" role="alert">{error}</p>{/if}
</IntegrationRow>

{#if showSkills}
  <T3SkillsDialog
    skills={importedSkills}
    excluded={connection?.excludedSkills ?? []}
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
  .int-status-line { margin: 10px 0 0; }
</style>
