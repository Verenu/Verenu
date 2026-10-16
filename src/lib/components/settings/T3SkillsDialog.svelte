<script lang="ts">
  import { onDestroy } from 'svelte';
  import { icons } from '../../icons';
  import { modalFocusTrap } from '../../modalFocus';
  import { portal } from '../../portal';
  import { modalBackdrop, modalCard, MOTION_PX, motionPx } from '../../motion';
  import { filterSkillNames, skillToken, sortedSkillNames } from './t3SkillNames';

  let {
    skills,
    busy = false,
    onpull,
    onclose,
    restoreFocus,
  }: {
    skills: { name: string }[];
    busy?: boolean;
    onpull: () => void;
    onclose: () => void;
    restoreFocus: () => HTMLElement | null;
  } = $props();

  const names = $derived(sortedSkillNames(skills));
  let query = $state('');
  let searchInput = $state<HTMLInputElement | null>(null);
  const visible = $derived(filterSkillNames(names, query));
  const noun = $derived(names.length === 1 ? 'skill' : 'skills');
  const countText = $derived(
    query.trim() && visible.length !== names.length
      ? `${visible.length} of ${names.length} ${noun}`
      : `${names.length} ${noun}`,
  );

  let copied = $state<{ token: string; ok: boolean } | null>(null);
  let copyTimer: ReturnType<typeof setTimeout> | undefined;
  const copyMessage = $derived(
    copied
      ? copied.ok
        ? `Copied ${copied.token}.`
        : `Could not copy ${copied.token}. Select the name and copy it manually.`
      : '',
  );

  onDestroy(() => clearTimeout(copyTimer));

  async function copy(name: string) {
    const token = skillToken(name);
    let ok = true;
    try {
      await navigator.clipboard.writeText(token);
    } catch {
      ok = false;
    }
    copied = { token, ok };
    clearTimeout(copyTimer);
    copyTimer = setTimeout(() => (copied = null), 2400);
  }

  function onWindowKeydown(event: KeyboardEvent) {
    if (event.key !== 'Escape') return;
    event.preventDefault();
    event.stopPropagation();
    if (query && event.target === searchInput) {
      query = '';
      return;
    }
    onclose();
  }
</script>

<svelte:window onkeydown={onWindowKeydown} />

<button
  type="button"
  class="ui-modal-backdrop skill-backdrop"
  aria-label="Close dialog"
  use:portal
  onclick={onclose}
  in:modalBackdrop={{ duration: 180 }}
  out:modalBackdrop={{ duration: 160 }}
></button>

<div
  class="ui-modal-card ui-modal-card--portalled skill-dialog"
  use:portal
  use:modalFocusTrap={{ active: true, initialFocus: () => searchInput, restoreFocus }}
  role="dialog"
  aria-modal="true"
  aria-labelledby="t3-skills-title"
  tabindex="-1"
  in:modalCard={{ duration: 220, distance: motionPx(MOTION_PX.panel), scaleFrom: 0.97 }}
  out:modalCard={{ duration: 160, distance: motionPx(MOTION_PX.nudge), scaleFrom: 0.985 }}
>
  <div class="ui-modal-head skill-head">
    <div class="skill-heading">
      <h2 id="t3-skills-title" class="ui-modal-title">Shared T3 skills</h2>
      <p class="skill-count-line" aria-live="polite">{countText}</p>
    </div>
    <button type="button" class="icon-btn" aria-label="Close" onclick={onclose}>
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true">{@html icons.close}</svg>
    </button>
  </div>

  {#if names.length}
    <div class="ui-modal-body skill-body">
      <input
        bind:this={searchInput}
        bind:value={query}
        type="search"
        class="ui-input"
        aria-label="Search skill names"
        placeholder="Search skill names"
        autocomplete="off"
        spellcheck="false"
      />
      <p class="copy-status" class:failed={copied !== null && !copied.ok} role="status">{copyMessage}</p>
      {#if visible.length}
        <ul class="skill-list scrollbar-standard" aria-label="Shared T3 skill names">
          {#each visible as name (name)}
            {@const token = skillToken(name)}
            {@const copiedThis = copied?.ok === true && copied.token === token}
            <li class="skill-row">
              <code>{token}</code>
              <button
                type="button"
                class="icon-btn copy-btn"
                class:copied={copiedThis}
                aria-label={`Copy ${token}`}
                title={`Copy ${token}`}
                onclick={() => copy(name)}
              >
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{@html copiedThis ? icons.check : icons.copy}</svg>
              </button>
            </li>
          {/each}
        </ul>
      {:else}
        <p class="empty">No skill names match “{query.trim()}”.</p>
      {/if}
    </div>
  {:else}
    <div class="ui-modal-body skill-body">
      <p class="empty">T3 Code has not shared any skills. Enable skills in T3 Code, then pull now.</p>
      <div>
        <button type="button" class="btn-primary" disabled={busy} onclick={onpull}>Pull now</button>
      </div>
    </div>
  {/if}
</div>

<style>
  .skill-backdrop { border: 0; cursor: default; padding: 0; z-index: 70; }
  .skill-dialog { display: flex; flex-direction: column; max-height: min(640px, calc(100vh - 40px)); width: min(440px, calc(100vw - 40px)); z-index: 71; }
  .skill-head { align-items: flex-start; display: flex; gap: 12px; justify-content: space-between; }
  .skill-heading { display: grid; gap: 4px; min-width: 0; }
  .skill-count-line { color: var(--ink-mute); font-size: 12px; font-variant-numeric: tabular-nums; margin: 0; }
  .skill-body { display: flex; flex: 1; flex-direction: column; gap: 10px; min-height: 0; }
  .skill-body .ui-input { width: 100%; }
  .skill-list { border-top: 1px solid var(--line-soft); flex: 1; list-style: none; margin: 0; max-height: 360px; min-height: 0; overflow-y: auto; padding: 0; }
  .skill-row { align-items: center; border-bottom: 1px solid var(--line-soft); display: flex; gap: 12px; justify-content: space-between; padding: 8px 2px; }
  .skill-row code { color: var(--ink); font-family: var(--mono); font-size: 12px; min-width: 0; overflow-wrap: anywhere; }
  .copy-btn.copied { color: var(--success); }
  .copy-status { color: var(--ink-mute); font-size: 12px; margin: 0; min-height: 18px; }
  .copy-status.failed { color: var(--danger); }
  .empty { color: var(--ink-soft); font-size: 13px; line-height: 1.5; margin: 0; }
  @media (max-width: 560px) {
    .skill-dialog {
      left: 0;
      right: 0;
      top: auto;
      bottom: 0;
      translate: 0 0;
      width: 100%;
      max-width: none;
      max-height: min(88vh, calc(100% - 24px));
      border-radius: var(--r-lg) var(--r-lg) 0 0;
      border-left: 0;
      border-right: 0;
      border-bottom: 0;
      padding-bottom: env(safe-area-inset-bottom, 0px);
    }
    .skill-list { max-height: none; }
  }
</style>
