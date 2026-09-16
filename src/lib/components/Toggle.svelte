<script lang="ts">
  let {
    checked = false,
    onchange,
    label = '',
    error = $bindable(false),
  }: {
    checked: boolean;
    onchange: (value: boolean) => void;
    label?: string;
    /** Set true to flash the toggle red with an X, e.g. when a save fails. Auto-resets itself. */
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
  role="switch"
  aria-checked={checked}
  aria-label={label || 'Toggle'}
  tabindex="0"
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
    transition: background 0.3s ease-out;
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
    transform: translateY(-50%);
    transition: left 0.35s cubic-bezier(0.22, 1, 0.36, 1);
    box-shadow: 0 1px 2px color-mix(in srgb, var(--ink) 15%, transparent);
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .toggle.on {
    background: var(--accent);
  }

  .toggle.on .toggle-thumb {
    left: 16px;
  }

  .toggle.error {
    /* Revert snaps instantly so the shake/X play on an already-settled
       toggle, instead of racing the position/background transitions. */
    transition: none;
    background: #ef4444;
    animation: toggle-shake 0.4s ease-out;
  }

  .toggle.error .toggle-thumb {
    transition: none;
  }

  .toggle-x {
    width: 9px;
    height: 9px;
    color: #ef4444;
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
