<script lang="ts">
  import type { Snippet } from 'svelte';

  let {
    name,
    label,
    target,
    busy = false,
    icon,
    description,
    actions,
    children,
  }: {
    name: string;
    label: string;
    target: string;
    busy?: boolean;
    icon: Snippet;
    description: Snippet;
    actions?: Snippet;
    children?: Snippet;
  } = $props();
</script>

<section class="integration" aria-label={label} aria-busy={busy} data-setting-target={target}>
  <header class="integration-head">
    <div class="integration-mark" aria-hidden="true">{@render icon()}</div>
    <div class="integration-title">
      <h3>{name}</h3>
      <p>{@render description()}</p>
    </div>
    {#if actions}<div class="integration-actions">{@render actions()}</div>{/if}
  </header>
  <div class="integration-content">{@render children?.()}</div>
</section>

<style>
  .integration { border-top: 1px solid var(--line); min-width: 0; padding: 20px 0; }
  .integration-head { align-items: flex-start; display: flex; gap: 14px; }
  .integration-mark { align-items: center; background: var(--paper-2); border: 1px solid var(--line); border-radius: 10px; display: flex; flex: 0 0 auto; height: 40px; justify-content: center; width: 40px; }
  .integration-mark :global(img) { height: 24px; width: 24px; }
  .integration-title { flex: 1; min-width: 0; }
  h3 { color: var(--ink-strong); font-size: 14px; font-weight: 600; letter-spacing: -0.005em; line-height: 1.3; margin: 0; }
  .integration-title p { color: var(--ink-mute); font-size: 12px; line-height: 1.5; margin: 2px 0 0; overflow-wrap: anywhere; }
  .integration-title p :global(code) { color: var(--ink-soft); font-family: var(--mono); font-size: 11.5px; }
  .integration-actions { align-items: center; display: flex; flex: 0 0 auto; gap: 2px; margin-right: -4px; }
  .integration-content { margin: 14px 0 0 54px; min-width: 0; }
  .integration-content:empty { display: none; }

  .integration :global(.int-status) { align-items: center; display: flex; flex-wrap: wrap; gap: 6px 14px; min-width: 0; }
  .integration :global(.int-state) { align-items: center; color: var(--ink-soft); display: inline-flex; font-size: 12.5px; gap: 7px; min-width: 0; overflow-wrap: anywhere; }
  .integration :global(.int-dot) { background: var(--success); border-radius: 50%; flex: 0 0 auto; height: 7px; width: 7px; }
  .integration :global(.int-dot.warn) { background: var(--warning); }
  .integration :global(.int-dot.bad) { background: var(--danger); }
  .integration :global(.int-spinner) { animation: int-spin 0.9s linear infinite; border: 1.5px solid currentColor; border-radius: 50%; border-right-color: transparent; flex: 0 0 auto; height: 9px; width: 9px; }
  .integration :global(.int-meta) { color: var(--ink-mute); font-size: 12px; min-width: 0; overflow-wrap: anywhere; }
  .integration :global(.int-link) { align-items: center; background: none; border: 0; border-radius: 4px; color: var(--ink); cursor: pointer; display: inline-flex; font-family: var(--sans); font-size: 12.5px; font-weight: 500; gap: 3px; padding: 0; text-align: left; }
  .integration :global(.int-link:hover) { color: var(--ink-strong); }
  .integration :global(.int-link svg) { transition: transform var(--ui-duration-fast) var(--ui-ease-out); }
  .integration :global(.int-link:hover svg) { transform: translateX(2px); }
  .integration :global(.int-note) { align-items: flex-start; color: var(--ink-soft); display: flex; font-size: 12px; gap: 8px; line-height: 1.5; margin: 10px 0 0; }
  .integration :global(.int-note::before) { background: var(--warning); border-radius: 50%; content: ''; flex: 0 0 6px; height: 6px; margin-top: 6px; }
  .integration :global(.int-fine) { color: var(--ink-faint); font-size: 11.5px; line-height: 1.5; margin: 12px 0 0; }
  .integration :global(.int-error) { color: var(--danger); font-size: 12px; line-height: 1.5; margin: 10px 0 0; overflow-wrap: anywhere; }
  .integration :global(.int-form label) { color: var(--ink-soft); display: block; font-size: 12px; margin: 0 0 7px; }
  .integration :global(.int-row) { align-items: stretch; display: flex; gap: 8px; }
  .integration :global(.int-row .ui-input) { flex: 1; min-height: 34px; min-width: 0; }
  .integration :global(.int-row .btn-primary) { flex: 0 0 auto; }
  .integration :global(.int-help) { color: var(--ink-mute); font-size: 11.5px; line-height: 1.5; margin: 8px 0 0; }
  .integration :global(.int-form .int-help.first) { margin: 0 0 10px; }
  .integration :global(.int-enter) { animation: int-enter var(--ui-duration-base) var(--ui-ease-out) both; }

  @keyframes int-spin { to { transform: rotate(360deg); } }
  @keyframes int-enter { from { opacity: 0; transform: translate3d(0, 5px, 0); } to { opacity: 1; transform: none; } }
  @media (prefers-reduced-motion: reduce) {
    .integration :global(.int-spinner) { animation-duration: 2.4s; }
    .integration :global(.int-enter) { animation: none; }
    .integration :global(.int-link svg) { transition: none; }
  }
  @container settings-panel (max-width: 440px) {
    .integration-content { margin-left: 0; }
    .integration :global(.int-row) { flex-direction: column; }
    .integration :global(.int-row .btn-primary) { min-height: 36px; }
  }
</style>
