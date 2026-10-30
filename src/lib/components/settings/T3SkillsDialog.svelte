<script lang="ts">
  import { flip } from 'svelte/animate';
  import { fly } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import { icons } from '../../icons';
  import { modalFocusTrap } from '../../modalFocus';
  import { portal } from '../../portal';
  import { modalBackdrop, modalCard, motionMs, motionPx, MOTION_MS, MOTION_PX } from '../../motion';
  import { setSkillsExcluded } from '../../t3Store.svelte';
  import { filterSkillNames, isSkillExcluded, skillToken, sortedSkillNames } from './t3SkillNames';

  let {
    skills,
    excluded,
    busy = false,
    onpull,
    onclose,
    restoreFocus,
  }: {
    skills: { name: string }[];
    excluded: readonly string[];
    busy?: boolean;
    onpull: () => void;
    onclose: () => void;
    restoreFocus: () => HTMLElement | null;
  } = $props();

  const names = $derived(sortedSkillNames(skills));
  let query = $state('');
  let searchInput = $state<HTMLInputElement | null>(null);
  let list = $state<HTMLElement | null>(null);
  let fadeTop = $state(0);
  let fadeBottom = $state(0);
  const visible = $derived(filterSkillNames(names, query));
  const included = $derived(names.filter((name) => !isSkillExcluded(excluded, name)).length);
  const noun = $derived(names.length === 1 ? 'skill' : 'skills');
  const visibleIncluded = $derived(visible.filter((name) => !isSkillExcluded(excluded, name)).length);
  const allVisibleOn = $derived(visible.length > 0 && visibleIncluded === visible.length);

  function updateFades() {
    if (!list) return;
    const max = list.scrollHeight - list.clientHeight;
    fadeTop = Math.min(1, list.scrollTop / 24);
    fadeBottom = max <= 1 ? 0 : Math.min(1, (max - list.scrollTop) / 24);
  }

  $effect(() => {
    void visible.length;
    if (!list) return;
    requestAnimationFrame(updateFades);
  });

  function toggle(name: string, on: boolean) {
    void setSkillsExcluded([name], !on);
  }

  function toggleVisible() {
    void setSkillsExcluded(visible, allVisibleOn);
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
  class:empty-dialog={!names.length}
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
      <p class="skill-count-line" aria-live="polite">{included} of {names.length} {noun} sent to cleanup</p>
    </div>
    <button type="button" class="icon-btn" aria-label="Close" onclick={onclose}>
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true">{@html icons.close}</svg>
    </button>
  </div>

  {#if names.length}
    <div class="ui-modal-body skill-body">
      <div class="skill-tools">
        <div class="search-field">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" aria-hidden="true"><circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/></svg>
          <input
            bind:this={searchInput}
            bind:value={query}
            type="search"
            aria-label="Search skill names"
            placeholder="Search skill names"
            autocomplete="off"
            spellcheck="false"
          />
        </div>
        <button type="button" class="btn-ghost btn-compact bulk-btn" disabled={!visible.length} onclick={toggleVisible}>
          {allVisibleOn ? 'Clear' : 'Select all'}{query.trim() && visible.length ? ` (${visible.length})` : ''}
        </button>
      </div>

      <div class="list-frame" style:--fade-top={fadeTop} style:--fade-bottom={fadeBottom}>
        <ul class="skill-list scrollbar-standard" aria-label="Shared T3 skill names" bind:this={list} onscroll={updateFades}>
          {#each visible as name (name)}
            {@const token = skillToken(name)}
            {@const on = !isSkillExcluded(excluded, name)}
            <li
              class="skill-row"
              animate:flip={{ duration: motionMs(MOTION_MS.base), easing: cubicOut }}
              in:fly={{ y: 4, duration: motionMs(MOTION_MS.fast), easing: cubicOut }}
            >
              <label class:off={!on}>
                <input type="checkbox" class="skill-check" checked={on} onchange={(event) => toggle(name, event.currentTarget.checked)} />
                <span class="box" aria-hidden="true">
                  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3.2" stroke-linecap="round" stroke-linejoin="round"><path d="M5 12.5 10 17.5 19 7"/></svg>
                </span>
                <code>{token}</code>
              </label>
            </li>
          {/each}
        </ul>
        {#if !visible.length}
          <p class="empty" in:fly={{ y: 4, duration: motionMs(MOTION_MS.fast), easing: cubicOut }}>No skill names match “{query.trim()}”.</p>
        {/if}
      </div>
      <p class="skill-hint">Unchecked skills are never offered to your cleanup provider.</p>
    </div>
  {:else}
    <div class="ui-modal-body skill-body">
      <p class="empty static">T3 Code has not shared any skills. Enable skills in T3 Code, then pull now.</p>
      <div>
        <button type="button" class="btn-primary" disabled={busy} onclick={onpull}>Pull now</button>
      </div>
    </div>
  {/if}
</div>

<style>
  .skill-backdrop { border: 0; cursor: default; padding: 0; z-index: 70; }
  .skill-dialog { --ui-modal-w: 460px; display: flex; flex-direction: column; height: min(600px, calc(100vh - 40px)); z-index: 71; }
  .skill-dialog.empty-dialog { height: auto; }
  .skill-head { align-items: flex-start; display: flex; gap: 12px; justify-content: space-between; }
  .skill-heading { display: grid; gap: 4px; min-width: 0; }
  .skill-count-line { color: var(--ink-mute); font-size: 12px; font-variant-numeric: tabular-nums; margin: 0; }
  .skill-body { display: flex; flex: 1; flex-direction: column; gap: 10px; min-height: 0; overflow: hidden; padding-top: 14px; }
  .skill-tools { align-items: stretch; display: flex; gap: 8px; }
  .search-field { align-items: center; background: var(--paper); border: 1px solid var(--line); border-radius: var(--r-sm); color: var(--ink-faint); display: flex; flex: 1; gap: 8px; min-width: 0; padding: 0 11px; transition: border-color var(--ui-duration-fast) var(--ui-ease-out), color var(--ui-duration-fast) var(--ui-ease-out); }
  .search-field:focus-within { border-color: var(--line-strong); color: var(--ink-soft); }
  .search-field input { background: transparent; border: 0; color: var(--ink); font-family: var(--sans); font-size: 13px; height: 34px; min-width: 0; outline: none; padding: 0; width: 100%; }
  .search-field input::placeholder { color: var(--ink-faint); }
  .search-field input::-webkit-search-cancel-button { appearance: none; }
  .bulk-btn { flex: 0 0 auto; font-variant-numeric: tabular-nums; }
  .list-frame { flex: 1; min-height: 0; position: relative; }
  .skill-list {
    --mask-top: calc(var(--fade-top, 0) * 22px);
    --mask-bottom: calc(var(--fade-bottom, 0) * 28px);
    -webkit-mask-image: linear-gradient(to bottom, transparent 0, #000 var(--mask-top), #000 calc(100% - var(--mask-bottom)), transparent 100%);
    mask-image: linear-gradient(to bottom, transparent 0, #000 var(--mask-top), #000 calc(100% - var(--mask-bottom)), transparent 100%);
    height: 100%;
    list-style: none;
    margin: 0;
    overflow-y: auto;
    padding: 0;
  }
  .skill-row { border-bottom: 1px solid var(--line-soft); }
  .skill-row:last-child { border-bottom: 0; }
  .skill-row label { align-items: center; border-radius: var(--r-sm); cursor: pointer; display: flex; gap: 12px; padding: 9px 6px; position: relative; transition: background-color var(--ui-duration-fast) var(--ui-ease-out); }
  .skill-row label:hover { background: var(--control-hover); }
  .skill-row code { color: var(--ink); font-family: var(--mono); font-size: 12px; min-width: 0; overflow-wrap: anywhere; transition: color var(--ui-duration-fast) var(--ui-ease-out), opacity var(--ui-duration-fast) var(--ui-ease-out); }
  .skill-row label.off code { color: var(--ink-mute); opacity: 0.7; text-decoration: line-through; text-decoration-color: color-mix(in srgb, var(--ink-mute) 50%, transparent); }
  .skill-check { cursor: pointer; height: 100%; inset: 0; margin: 0; opacity: 0; position: absolute; width: 100%; z-index: 1; }
  .box { align-items: center; background: var(--paper); border: 1px solid var(--line-strong); border-radius: 5px; color: var(--on-accent); display: inline-flex; flex: 0 0 auto; height: 18px; justify-content: center; transition: background-color var(--ui-duration-fast) var(--ui-ease-out), border-color var(--ui-duration-fast) var(--ui-ease-out), transform var(--ui-duration-fast) var(--ui-ease-out); width: 18px; }
  .box svg { height: 12px; stroke-dasharray: 22; stroke-dashoffset: 22; transition: stroke-dashoffset var(--ui-duration-base) var(--ui-ease-out); width: 12px; }
  .skill-check:checked + .box { background: var(--ink); border-color: var(--ink); color: var(--paper); }
  .skill-check:checked + .box svg { stroke-dashoffset: 0; }
  .skill-row label:active .box { transform: scale(0.9); }
  .skill-check:focus-visible + .box { box-shadow: var(--ui-focus-ring); outline: 2px solid var(--accent); outline-offset: 2px; }
  .empty { color: var(--ink-mute); font-size: 13px; left: 0; line-height: 1.5; margin: 0; padding: 18px 6px; position: absolute; right: 0; top: 0; }
  .empty.static { color: var(--ink-soft); padding: 0; position: static; }
  .skill-hint { color: var(--ink-faint); font-size: 11.5px; line-height: 1.5; margin: 0; }
  @media (prefers-reduced-motion: reduce) {
    .box svg, .box, .skill-row label, .skill-row code { transition: none; }
  }
  @media (max-width: 560px) {
    .skill-dialog {
      left: 0;
      right: 0;
      top: auto;
      bottom: 0;
      translate: 0 0;
      width: 100%;
      max-width: none;
      height: min(88vh, calc(100% - 24px));
      border-radius: var(--r-lg) var(--r-lg) 0 0;
      border-left: 0;
      border-right: 0;
      border-bottom: 0;
      padding-bottom: env(safe-area-inset-bottom, 0px);
    }
    .skill-dialog.empty-dialog { height: auto; }
  }
</style>
