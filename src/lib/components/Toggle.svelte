<script lang="ts">
  let {
    checked = false,
    onchange,
    label = '',
    disabled = false,
    gated = false,
    error = $bindable(false),
  }: {
    checked: boolean;
    onchange: (value: boolean) => void;
    label?: string;
    disabled?: boolean;
    /** Looks and reads as disabled, but still takes a tap so the caller can open the accept flow. */
    gated?: boolean;
    error?: boolean;
  } = $props();

  $effect(() => {
    if (!error) return;
    const timer = setTimeout(() => (error = false), 650);
    return () => clearTimeout(timer);
  });
</script>

<button
  type="button"
  class="toggle"
  class:on={checked}
  class:error
  class:gated
  aria-disabled={gated || undefined}
  role="switch"
  aria-checked={checked}
  aria-label={label || 'Toggle'}
  tabindex="0"
  {disabled}
  onclick={() => onchange(!checked)}
>
  <span class="toggle-thumb" aria-hidden="true">
    {#if error}
      <svg class="toggle-x" viewBox="0 0 12 12" aria-hidden="true">
        <path d="M3 3l6 6M9 3L3 9" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
      </svg>
    {/if}
  </span>
</button>

<style>
  .toggle {
    width: 30px;
    height: 16px;
    display: block;
    box-sizing: border-box;
    padding: 0;
    border: 0;
    appearance: none;
    -webkit-appearance: none;
    background: var(--line-strong);
    border-radius: 999px;
    position: relative;
    cursor: pointer;
    transition: background-color var(--ui-duration-base) var(--ui-ease-out);
    flex-shrink: 0;
  }

  .toggle-thumb {
    position: absolute;
    width: 12px;
    height: 12px;
    background: var(--bg-elev);
    border-radius: 50%;
    top: 50%;
    left: 2px;
    transform: translate3d(0, -50%, 0);
    transition: transform var(--ui-duration-base) var(--ui-ease-out);
    box-shadow: 0 1px 2px color-mix(in srgb, var(--ink) 15%, transparent);
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .toggle.gated { opacity: 0.5; }

  .toggle.on {
    background: var(--accent);
  }

  .toggle:disabled { opacity: 0.45; cursor: default; }

  .toggle.on .toggle-thumb {
    transform: translate3d(14px, -50%, 0);
  }

  .toggle.error {
    transition: none;
    background: var(--danger);
    animation: toggle-shake 0.4s ease-out;
  }

  .toggle.error .toggle-thumb {
    transition: none;
  }

  .toggle-x {
    width: 9px;
    height: 9px;
    color: var(--danger);
    animation: toggle-x-in 0.25s cubic-bezier(0.22, 1, 0.36, 1);
  }

  .toggle:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }

  @keyframes toggle-shake {
    0%, 100% { transform: translateX(0); }
    20% { transform: translateX(-2px); }
    40% { transform: translateX(2px); }
    60% { transform: translateX(-1px); }
    80% { transform: translateX(1px); }
  }

  @keyframes toggle-x-in {
    from { transform: translateX(-6px); opacity: 0; }
    to { transform: translateX(0); opacity: 1; }
  }
</style>
