<script lang="ts">
  import { scale } from 'svelte/transition';
  import { flip } from 'svelte/animate';
  import { cubicOut } from 'svelte/easing';
  import { MOTION_MS, motionMs } from '../../motion';

  let {
    id,
    label,
    values = $bindable([]),
    draft = $bindable(''),
    placeholder = '',
    suggestions = [],
    disabled = false,
  }: {
    id: string;
    label: string;
    values: string[];
    /** Text typed but not yet turned into a tag; the parent folds it in on save. */
    draft: string;
    placeholder?: string;
    suggestions?: string[];
    disabled?: boolean;
  } = $props();

  let input: HTMLInputElement | undefined;
  const open = $derived(suggestions.filter(s => !values.includes(s)));

  function add(text: string) {
    const next = text.split(/[\n,]/).map(x => x.trim()).filter(Boolean);
    if (!next.length) return;
    values = [...new Set([...values, ...next])];
  }
  function commit() { add(draft); draft = ''; }
  function remove(model: string) { values = values.filter(v => v !== model); input?.focus(); }
  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' || e.key === ',') {
      // Enter adds another ID instead of submitting the whole form.
      e.preventDefault();
      commit();
    } else if (e.key === 'Backspace' && !draft && values.length) {
      values = values.slice(0, -1);
    }
  }
  function onpaste(e: ClipboardEvent) {
    const text = e.clipboardData?.getData('text') ?? '';
    if (!/[\n,]/.test(text)) return;
    e.preventDefault();
    add(draft + text);
    draft = '';
  }
  const dur = () => motionMs(MOTION_MS.fast);
</script>

<div class="mid">
  <label for={id}>{label}</label>
  <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
  <div class="box" role="presentation" class:disabled onclick={() => input?.focus()}>
    {#each values as v (v)}
      <span class="tag" animate:flip={{ duration: dur(), easing: cubicOut }} in:scale={{ start: 0.85, duration: dur(), easing: cubicOut }} out:scale={{ start: 0.85, duration: motionMs(100) }}>
        <span class="tag-text">{v}</span>
        <button type="button" class="tag-x" aria-label="Remove {v}" onclick={e => { e.stopPropagation(); remove(v); }} {disabled}>
          <svg viewBox="0 0 10 10" width="8" height="8" aria-hidden="true"><path d="M2 2l6 6M8 2L2 8" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" fill="none"/></svg>
        </button>
      </span>
    {/each}
    <input {id} bind:this={input} bind:value={draft} {onkeydown} {onpaste} onblur={commit} {disabled} autocomplete="off" spellcheck="false" autocapitalize="off"
      placeholder={values.length ? 'Add another' : placeholder} />
  </div>
  <small>Type an ID and press Enter. Add as many as you like.</small>
  {#if open.length}
    <span class="suggest">
      <span class="suggest-label">Suggested</span>
      {#each open as s (s)}
        <button type="button" class="chip" onclick={() => add(s)} {disabled} transition:scale={{ start: 0.9, duration: dur() }}>
          <svg viewBox="0 0 10 10" width="8" height="8" aria-hidden="true"><path d="M5 1.5v7M1.5 5h7" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" fill="none"/></svg>{s}
        </button>
      {/each}
    </span>
  {/if}
</div>

<style>
  .mid { display: flex; flex-direction: column; gap: 7px; min-width: 0; font-size: 12px; color: var(--ink-soft); }
  .box { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; min-height: 38px; box-sizing: border-box; padding: 6px 8px; border: 1px solid var(--line); border-radius: 8px; background: transparent; cursor: text; transition: border-color 120ms ease; }
  .box:focus-within { outline: 2px solid var(--accent); outline-offset: 2px; }
  .box.disabled { opacity: 0.6; cursor: default; }
  .tag { display: inline-flex; align-items: center; gap: 4px; max-width: 100%; padding: 3px 4px 3px 9px; border-radius: 6px; background: var(--paper-2); color: var(--ink); font-family: var(--mono); font-size: 11.5px; }
  .tag-text { overflow-wrap: anywhere; }
  .tag-x { display: grid; place-items: center; width: 16px; height: 16px; padding: 0; border: 0; border-radius: 5px; background: transparent; color: var(--ink-mute); cursor: pointer; }
  .tag-x:hover:not(:disabled) { background: var(--control-active); color: var(--ink); }
  .tag-x:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }
  input { flex: 1 1 110px; min-width: 90px; width: auto; padding: 3px 2px; border: 0; background: transparent; color: var(--ink); font-family: var(--mono); font-size: 12px; outline: none; }
  small { font-size: 11px; line-height: 1.5; color: var(--ink-mute); }
  .suggest { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; }
  .suggest-label { font-size: 11px; color: var(--ink-mute); }
  .chip { display: inline-flex; align-items: center; gap: 6px; max-width: 100%; padding: 4px 9px; border: 1px solid var(--line); border-radius: 6px; background: transparent; color: var(--ink-soft); font-family: var(--mono); font-size: 11px; cursor: pointer; overflow-wrap: anywhere; text-align: left; transition: background 120ms ease, color 120ms ease; }
  .chip:hover:not(:disabled) { background: var(--control-hover); color: var(--ink); }
  .chip:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
</style>
