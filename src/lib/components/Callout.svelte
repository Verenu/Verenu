<script lang="ts" module>
  export type CalloutSeverity = 'info' | 'warning' | 'error';

  export type CalloutAction = {
    label: string;
    onclick: (event: MouseEvent) => void;
    disabled?: boolean;
  };
</script>

<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HTMLAttributes } from 'svelte/elements';

  let {
    severity = 'warning',
    title = '',
    action,
    secondaryAction,
    onDismiss,
    dismissLabel = 'Dismiss',
    compact = false,
    role,
    iconHtml,
    children,
    class: className = '',
    ...rest
  }: {
    severity?: CalloutSeverity;
    title?: string;
    action?: CalloutAction;
    secondaryAction?: CalloutAction;
    onDismiss?: () => void;
    dismissLabel?: string;
    compact?: boolean;
    role?: HTMLAttributes<HTMLDivElement>['role'];
    /** Trusted inline SVG that replaces the severity icon, such as a provider logo. */
    iconHtml?: string | null;
    children?: Snippet;
    class?: string;
  } & Omit<HTMLAttributes<HTMLDivElement>, 'role' | 'class' | 'title' | 'children'> = $props();

  const resolvedRole = $derived(role !== undefined ? role : severity === 'error' ? 'alert' : 'status');
  const hasActions = $derived(!!action || !!secondaryAction || !!onDismiss);
</script>

<div
  class={`callout callout-${severity} ${className}`.trim()}
  class:callout-compact={compact}
  class:callout-has-actions={hasActions}
  role={resolvedRole ?? undefined}
  {...rest}
>
  <div class="callout-main">
    <span class="callout-icon" aria-hidden="true">
      {#if iconHtml}
        {@html iconHtml}
      {:else if severity === 'error'}
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9.5" /><path d="M12 7.5v5.5" /><path d="M12 16.5h.01" /></svg>
      {:else if severity === 'warning'}
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M10.3 3.9 1.8 18.2A2 2 0 0 0 3.5 21h17a2 2 0 0 0 1.7-2.8L13.7 3.9a2 2 0 0 0-3.4 0Z" /><path d="M12 9.5v4.5" /><path d="M12 17.5h.01" /></svg>
      {:else}
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9.5" /><path d="M12 11v5.5" /><path d="M12 7.5h.01" /></svg>
      {/if}
    </span>
    <div class="callout-body">
      {#if title}<strong class="callout-title">{title}</strong>{/if}
      {#if children}<div class="callout-message">{@render children()}</div>{/if}
    </div>
  </div>
  {#if hasActions}
    <div class="callout-actions">
      {#if secondaryAction}
        <button type="button" class="btn-ghost btn-compact" disabled={secondaryAction.disabled} onclick={secondaryAction.onclick}>{secondaryAction.label}</button>
      {/if}
      {#if action}
        <button type="button" class="btn-ghost btn-compact callout-action" disabled={action.disabled} onclick={action.onclick}>{action.label}</button>
      {/if}
      {#if onDismiss}
        <button type="button" class="callout-dismiss ui-focus-ring" aria-label={dismissLabel} onclick={onDismiss}>
          <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" aria-hidden="true"><path d="M6 6l12 12M6 18 18 6" /></svg>
        </button>
      {/if}
    </div>
  {/if}
</div>

<style>
  .callout {
    --callout-tint: var(--ink-mute);
    --callout-line-height: 19px;
    --callout-control: 28px;
    display: flex;
    flex-wrap: wrap;
    align-items: flex-start;
    gap: 6px 12px;
    box-sizing: border-box;
    width: 100%;
    padding: 10px 14px;
    border: 1px solid var(--line);
    border-radius: var(--r-md);
    background: var(--paper-2);
    color: var(--ink-soft);
    font-family: var(--sans);
    font-size: 12.5px;
    line-height: var(--callout-line-height);
    text-align: left;
  }
  .callout-compact {
    --callout-line-height: 17px;
    --callout-control: 26px;
    padding: 8px 12px;
    border-radius: var(--r-sm);
    font-size: 12px;
  }

  .callout-warning {
    --callout-tint: var(--warning);
    border-color: var(--warning-line);
    background: var(--warning-bg);
  }
  .callout-error {
    --callout-tint: var(--danger);
    border-color: var(--danger-line);
    background: var(--danger-bg);
  }

  .callout-icon {
    display: inline-flex;
    flex: 0 0 16px;
    align-items: center;
    justify-content: center;
    height: var(--callout-line-height);
    color: var(--callout-tint);
  }
  .callout-icon :global(svg) { width: 16px; height: 16px; }

  .callout-main {
    display: flex;
    flex: 1 1 220px;
    align-items: flex-start;
    gap: 12px;
    min-width: 0;
  }

  .callout-body {
    display: flex;
    flex: 1 1 0;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .callout-title { color: var(--ink); font-weight: 600; }
  .callout-message { color: var(--ink-soft); }
  .callout-message :global(strong) { color: var(--ink); font-weight: 600; }
  .callout-message :global(p) { margin: 0; }
  .callout-message :global(p + p) { margin-top: 4px; }

  /* Center the first text line on the first row of controls. */
  .callout-has-actions .callout-icon,
  .callout-has-actions .callout-body {
    padding-top: calc((var(--callout-control) - var(--callout-line-height)) / 2);
  }
  .callout-has-actions .callout-icon { height: auto; }

  .callout-actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    margin-left: auto;
    flex-shrink: 0;
  }
  .callout-warning .callout-actions .btn-ghost { border-color: var(--warning-line); }
  .callout-error .callout-actions .btn-ghost { border-color: var(--danger-line); }

  .callout-dismiss {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    padding: 0;
    border: 0;
    border-radius: var(--r-sm);
    background: transparent;
    color: var(--ink-mute);
    cursor: pointer;
  }
  .callout-dismiss:hover { background: var(--control-hover); color: var(--ink-strong); }
</style>
