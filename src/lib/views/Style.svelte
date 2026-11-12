<script lang="ts">
  import { invoke, emit } from '../tauri';
  import { onMount } from 'svelte';
  import { crossfade, fly } from 'svelte/transition';
  import { expoOut } from 'svelte/easing';
  import { saveSetting, type CleanupIntensity, type ToneId } from '../settings';
  import { appStore } from '../stores';
  import { contextsStore } from '../contextsStore.svelte';
  import { openCleanupPromptEditor } from '../stores.svelte';
  import { splitModelId } from '../components/settings/models';
  import { formatIpcError } from '../errors';
  import Callout from '../components/Callout.svelte';
  import { MOTION_MS, MOTION_PX, STYLE_TAB_ORDER, directionFromOrder, motionMs, motionPx, pageSwap } from '../motion';

  const [send, receive] = crossfade({
    duration: motionMs(MOTION_MS.base),
    easing: expoOut,
  });

  let tab = $state('cleanup');
  let tabDir = $state<1 | -1>(1);
  let mountedTabs = $state<Record<string, boolean>>({});
  let intensity = $state('medium');
  let tone = $state('casual');
  let editorError = $state('');
  const modelFreeCleanup = $derived(intensity === 'none' || intensity === 'rules');
  const toneInactive = $derived(modelFreeCleanup && !contextsStore.contexts.some(
    context => ['light', 'medium', 'high'].includes(context.cleanup_intensity ?? ''),
  ));

  async function editStyle(id: string, event: MouseEvent, isTone = false) {
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    editorError = '';
    try {
      const modelId = await invoke<string | null>('get_setting', { key: 'cleanup_default_model' });
      const target = splitModelId(modelId ?? 'groq/openai/gpt-oss-20b');
      if (!target) throw new Error('Choose a cleanup model in Settings first.');
      openCleanupPromptEditor(target.provider, target.model, rect,
        isTone ? null : id as 'light' | 'medium' | 'high',
        isTone ? id as ToneId : null);
    } catch (error) { editorError = formatIpcError(error, 'Could not open the prompt editor'); }
  }

  const tabs = [
    { id: 'cleanup', label: 'Cleanup' },
    { id: 'personal', label: 'Personal Tone' },
  ];

  const CLEANUP_INTRO = 'How much Verenu rewrites after transcription.';
  const TONE_INTRO = 'The default voice, used wherever a context group does not set its own tone.';

  const cleanupCards = [
    { id: 'none', name: 'Off', desc: 'Keep the raw transcript. Dual transcription may still reconcile candidates.', sample: "so um i was thinking like we should probably leave a bit earlier you know cause there's gonna be traffic i think" },
    { id: 'light', name: 'Light', desc: 'Remove non-semantic speech artifacts and fix basics. Keep wording, order, and detail.', sample: "I was thinking we should probably leave a bit earlier because there's going to be traffic, I think." },
    { id: 'medium', name: 'Medium', desc: 'Improve flow and remove redundancy with light restructuring. Preserve every distinct detail.', sample: "I think we should leave a bit earlier. There's going to be traffic." },
    { id: 'high', name: 'Strong', desc: 'Rewrite concisely and directly. Preserve facts, constraints, qualifiers, and emphasis.', sample: 'Leave early. There will be traffic.' },
  ];

  const personalCards = [
    { id: 'casual', name: 'Casual', desc: 'Contractions, normal casing and punctuation, and the speaker’s casual voice.', sample: "Hey, are you free for lunch tomorrow? Let's do 12 if that works." },
    { id: 'formal', name: 'Formal', desc: 'Professional wording and punctuation. No invented politeness or extra content.', sample: 'Are you available for lunch tomorrow? Let us meet at 12 if that works for you.' },
    { id: 'very_casual', name: 'Very Casual', desc: 'Mostly lowercase, contractions, and minimal readable punctuation. Preserve profanity and emphasis.', sample: "hey are you free for lunch tomorrow let's do 12 if that works" },
  ];

  onMount(async () => {
    try {
      const [savedTone, savedIntensity] = await Promise.all([
        invoke<string | null>('get_setting', { key: 'default_tone' }),
        invoke<string | null>('get_setting', { key: 'cleanup_intensity' }),
      ]);
      if (savedTone) tone = savedTone as string;
      if (savedIntensity) intensity = savedIntensity as string;
    } catch {
    }
  });

  function selectIntensity(id: string) {
    intensity = id;
    saveSetting('cleanup_intensity', id as CleanupIntensity);
  }

  function selectTone(id: string) {
    tone = id;
    saveSetting('default_tone', id as ToneId);
  }

  function selectTab(id: string) {
    if (id === tab) return;
    tabDir = directionFromOrder(tab, id, STYLE_TAB_ORDER);
    tab = id;
  }

  let tablistEl = $state<HTMLDivElement | null>(null);

  function handleTablistKeydown(event: KeyboardEvent) {
    const tabButtons = tablistEl?.querySelectorAll<HTMLButtonElement>('.tab') ?? [];
    if (tabButtons.length === 0) return;
    const index = Array.from(tabButtons).indexOf(document.activeElement as HTMLButtonElement);
    if (index === -1) return;

    let next: number | null = null;
    if (event.key === 'Home') next = 0;
    else if (event.key === 'End') next = tabButtons.length - 1;
    else if (event.key === 'ArrowLeft') next = (index - 1 + tabButtons.length) % tabButtons.length;
    else if (event.key === 'ArrowRight') next = (index + 1) % tabButtons.length;
    if (next === null) return;

    event.preventDefault();
    const target = tabButtons[next];
    target.focus();
    const id = target.id.replace('style-tab-', '');
    if (id !== tab) {
      tabDir = directionFromOrder(tab, id, STYLE_TAB_ORDER);
      tab = id;
    }
  }

  $effect(() => {
    if (!mountedTabs[tab]) {
      mountedTabs = { ...mountedTabs, [tab]: true };
    }
  });
</script>

{#snippet cardGrid(kind: 'cleanup' | 'personal', enter: boolean)}
  {@const cards = kind === 'cleanup' ? cleanupCards : personalCards}
  {@const selected = kind === 'cleanup' ? intensity : tone}
  <div
    class="style-grid"
    class:four={kind === 'cleanup'}
    class:tab-content-disabled={kind === 'personal' && toneInactive}
    inert={kind === 'personal' && toneInactive}
  >
    {#each cards as c (c.id)}
      <div class="style-card-wrap">
        <button
          type="button"
          class="style-card"
          class:active={selected === c.id}
          aria-pressed={selected === c.id}
          onclick={() => (kind === 'cleanup' ? selectIntensity(c.id) : selectTone(c.id))}
          in:fly={enter ? { y: motionPx(MOTION_PX.lift), duration: motionMs(MOTION_MS.panel), easing: expoOut } : undefined}
        >
          <span class="style-card-head">
            <span class="style-radio" aria-hidden="true"></span>
            <span class="style-card-title">{c.name}</span>
          </span>
          <span class="desc">{c.desc}</span>
          <span class="style-sample">“{c.sample}”</span>
        </button>
        {#if kind === 'personal' || c.id !== 'none'}
          <button
            class="style-edit"
            aria-label={kind === 'cleanup' ? `Edit ${c.name} cleanup prompt` : `Edit ${c.name} tone instructions`}
            onclick={(event) => editStyle(c.id, event, kind === 'personal')}
          >
            <svg aria-hidden="true" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="m15 5 4 4M4 20l4-1L20 7a2.8 2.8 0 0 0-4-4L4 15z" /></svg>
          </button>
        {/if}
      </div>
    {/each}
  </div>
{/snippet}

<div class="content-inner">
  <header class="style-head">
    <h1 class="page-h">Style</h1>
    <p class="page-sub">How Verenu shapes your dictation.</p>
  </header>

  {#if editorError || !appStore.cleanupEnabled || modelFreeCleanup}
    <div class="style-notices">
      {#if editorError}
        <Callout severity="error" onDismiss={() => (editorError = '')}>{editorError}</Callout>
      {/if}
      {#if !appStore.cleanupEnabled}
        <Callout
          severity="info"
          title="Cleanup is turned off"
          action={{ label: 'Turn cleanup on', onclick: () => emit('open-flow:open-settings-section', 'text') }}
        >Nothing on this page has any effect right now. Tone, intensity, and app-specific overrides only apply during the cleanup step. Your choices below are kept, just not used.</Callout>
      {/if}
      {#if intensity === 'rules'}
        <Callout severity="info">Your previous Basic cleanup setting is still active on this device. Choose a cleanup level below to replace it; Basic cannot be selected again.</Callout>
      {/if}
      {#if modelFreeCleanup}
        <Callout severity="info">Tone and custom AI instructions do not apply to the current cleanup setting. Contexts using AI cleanup can still use the default tone.</Callout>
      {/if}
    </div>
  {/if}

  {#if appStore.legacyFeaturesEnabled}
    <div class="tabs" role="tablist" tabindex="-1" bind:this={tablistEl} onkeydown={handleTablistKeydown}>
      {#each tabs as t}
        <button
          class="tab ui-focus-ring"
          class:active={tab === t.id}
          role="tab"
          id="style-tab-{t.id}"
          tabindex={tab === t.id ? 0 : -1}
          aria-selected={tab === t.id}
          aria-controls="style-panel-{t.id}"
          onclick={() => selectTab(t.id)}
        >
          {t.label}
          {#if tab === t.id}
            <div class="active-bar" in:receive={{key: 'tab'}} out:send={{key: 'tab'}}></div>
          {/if}
        </button>
      {/each}
    </div>

    <div class="tab-content-area" class:tab-content-disabled={!appStore.cleanupEnabled} aria-disabled={!appStore.cleanupEnabled} inert={!appStore.cleanupEnabled}>
      {#key tab}
        <div
          class="tab-wrapper"
          role="tabpanel"
          id="style-panel-{tab}"
          aria-labelledby="style-tab-{tab}"
          in:pageSwap={{ axis: 'x', distance: tabDir * motionPx(MOTION_PX.panel), duration: motionMs(MOTION_MS.panel) }}
          out:pageSwap={{ axis: 'x', distance: -tabDir * motionPx(MOTION_PX.panel), duration: motionMs(MOTION_MS.base + 40) }}
        >
          {#if tab === 'cleanup'}
            <p class="style-intro">{CLEANUP_INTRO}</p>
            {@render cardGrid('cleanup', !mountedTabs.cleanup)}
          {:else if tab === 'personal'}
            <p class="style-intro">{TONE_INTRO}</p>
            {@render cardGrid('personal', !mountedTabs.personal)}
          {/if}
        </div>
      {/key}
    </div>
  {:else}
    <div class="style-sections" class:tab-content-disabled={!appStore.cleanupEnabled} aria-disabled={!appStore.cleanupEnabled} inert={!appStore.cleanupEnabled}>
      <section class="style-section">
        <div class="style-section-head">
          <h2 class="style-section-h">Cleanup</h2>
          <p class="style-intro">{CLEANUP_INTRO}</p>
        </div>
        {@render cardGrid('cleanup', true)}
      </section>

      <section class="style-section">
        <div class="style-section-head">
          <h2 class="style-section-h">Personal tone</h2>
          <p class="style-intro">{TONE_INTRO}</p>
        </div>
        {@render cardGrid('personal', true)}
      </section>
    </div>
  {/if}
</div>

<style>
  .content-inner {
    width: min(100%, var(--page-column));
    margin-inline: auto;
    padding: var(--page-pad-y) var(--page-pad-x) 36px;
    min-width: 0;
    position: relative;
    display: flex;
    flex-direction: column;
    min-height: 100%;
    container-type: inline-size;
    container-name: style-page;
  }

  .style-head { margin-bottom: 24px; }

  :global(.app.app-linux) .style-head {
    margin-top: var(--native-chrome-below-clearance);
  }

  .page-h {
    font-family: var(--sans);
    font-size: 23px;
    font-weight: 600;
    letter-spacing: -0.025em;
    margin: 0 0 4px;
    line-height: 1.1;
    color: var(--ink);
  }

  .page-sub { color: var(--ink-mute); font-size: 12.5px; margin: 0; }

  .style-notices {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-bottom: 24px;
  }

  .tab-content-area {
    position: relative;
    flex: 1;
    display: grid;
  }

  .tab-content-disabled {
    opacity: 0.45;
    pointer-events: none;
    user-select: none;
  }

  .tab-wrapper { grid-area: 1 / 1; }

  .style-sections {
    display: flex;
    flex-direction: column;
    gap: 26px;
  }

  .style-section-head {
    display: flex;
    align-items: baseline;
    gap: 4px 12px;
    flex-wrap: wrap;
    padding-bottom: 9px;
    margin-bottom: 14px;
    border-bottom: 1px solid var(--line);
  }

  .style-section-h {
    font-family: var(--sans);
    font-size: 14px;
    font-weight: 550;
    letter-spacing: -0.01em;
    color: var(--ink);
    margin: 0;
  }

  .style-intro {
    font-size: 12.5px;
    line-height: 1.45;
    color: var(--ink-mute);
    margin: 0;
  }

  .tab-wrapper > .style-intro { margin-bottom: 16px; }

  .tabs {
    display: flex;
    flex-wrap: wrap;
    gap: 22px;
    border-bottom: 1px solid var(--line);
    margin-bottom: 18px;
  }

  .tab {
    padding: 0 0 11px;
    font-size: 13px;
    color: var(--ink-mute);
    border: 0;
    background: transparent;
    display: flex;
    align-items: center;
    gap: 6px;
    cursor: pointer;
    position: relative;
    transition: color var(--ui-duration-fast) var(--ui-ease-out);
  }

  .tab:hover { color: var(--ink-soft); }
  .tab.active { color: var(--ink); font-weight: 500; }

  .active-bar {
    position: absolute;
    bottom: -1px;
    left: 0;
    right: 0;
    height: 1px;
    background: var(--ink);
  }

  .style-grid {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    column-gap: 10px;
  }

  .style-grid.four { grid-template-columns: repeat(4, minmax(0, 1fr)); }

  @container style-page (max-width: 760px) {
    .style-grid,
    .style-grid.four { grid-template-columns: repeat(2, minmax(0, 1fr)); }
  }

  @container style-page (max-width: 500px) {
    .style-grid,
    .style-grid.four { grid-template-columns: 1fr; }
  }

  /* Each card spans three shared rows (name, description, sample), so the samples line up across a row of cards. */
  .style-card-wrap,
  .style-card {
    display: grid;
    grid-template-rows: subgrid;
    grid-row: span 3;
    row-gap: 0;
  }

  .style-card-wrap { position: relative; min-width: 0; margin-bottom: 10px; }

  .style-card {
    border: 1px solid var(--line);
    width: 100%;
    text-align: left;
    padding: 14px 14px 15px;
    background: var(--bg-elev);
    border-radius: var(--r-md);
    cursor: pointer;
    transition:
      background-color var(--ui-duration-fast) var(--ui-ease-out),
      border-color var(--ui-duration-fast) var(--ui-ease-out);
  }

  .style-card:hover {
    background: var(--control-hover);
    border-color: var(--line-strong);
  }

  .style-card:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
    box-shadow: var(--ui-focus-ring);
  }

  .style-card.active {
    background: var(--control-active);
    border-color: color-mix(in srgb, var(--accent) 55%, var(--line-strong));
  }

  .style-card-head {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 7px;
    padding-right: 26px;
  }

  /* The cards are a single choice, so each one carries the radio mark that says so. */
  .style-radio {
    flex: none;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    border: 1.5px solid var(--line-strong);
    display: grid;
    place-items: center;
    transition: border-color var(--ui-duration-fast) var(--ui-ease-out);
  }

  .style-radio::after {
    content: '';
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--accent);
    opacity: 0;
    transform: scale(0.4);
    transition:
      opacity var(--ui-duration-fast) var(--ui-ease-out),
      transform var(--ui-duration-fast) var(--ui-ease-out);
  }

  .style-card.active .style-radio { border-color: var(--accent); }
  .style-card.active .style-radio::after { opacity: 1; transform: none; }

  .style-card-title {
    font-family: var(--sans);
    font-size: 14px;
    font-weight: 550;
    letter-spacing: -0.01em;
    line-height: 1.2;
    color: var(--ink);
  }

  .style-card .desc {
    display: block;
    font-size: 12px;
    color: var(--ink-mute);
    line-height: 1.45;
    margin-bottom: 14px;
  }

  /* What the same sentence looks like at this setting. It sits at the foot of every card so the row compares at a glance. */
  .style-sample {
    display: block;
    align-self: start;
    padding-top: 12px;
    border-top: 1px solid var(--line);
    font-family: var(--sans);
    font-size: 13px;
    line-height: 1.5;
    color: var(--ink-soft);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    transition: color var(--ui-duration-fast) var(--ui-ease-out);
  }

  .style-card.active .style-sample { color: var(--ink); }

  .style-edit {
    position: absolute;
    top: 9px;
    right: 9px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 26px;
    padding: 0;
    background: transparent;
    border: 0;
    border-radius: 6px;
    color: var(--ink-mute);
    cursor: pointer;
    opacity: 0;
    transition:
      opacity var(--ui-duration-fast) var(--ui-ease-out),
      background-color var(--ui-duration-fast) var(--ui-ease-out),
      color var(--ui-duration-fast) var(--ui-ease-out);
  }

  .style-edit:hover { background: var(--paper-3); color: var(--ink-strong); }
  .style-edit:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; box-shadow: var(--ui-focus-ring); }
  .style-card-wrap:hover .style-edit, .style-card-wrap:focus-within .style-edit { opacity: 1; }
  @media (hover: none) { .style-edit { opacity: 1; } }

  @media (max-width: 720px) {
    .tabs { gap: 14px; }
  }
</style>
