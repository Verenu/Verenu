<script lang="ts">
  import { fly } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import { MOTION_MS, MOTION_PX, motionMs, motionPx } from '../../motion';
  import { themeEditor } from '../../themeEditor.svelte';
  import ThemeEditorPanel from './ThemeEditorPanel.svelte';
</script>

{#if themeEditor.open}
  {#key themeEditor.session}
    <div
      class="theme-dock"
      in:fly={{ y: motionPx(MOTION_PX.panel), duration: motionMs(MOTION_MS.panel), easing: cubicOut }}
      out:fly={{ y: motionPx(MOTION_PX.nudge), duration: motionMs(MOTION_MS.fast), easing: cubicOut }}
    >
      <ThemeEditorPanel />
    </div>
  {/key}
{/if}

<style>
  .theme-dock {
    background: var(--bg-elev);
    border: 1px solid var(--line-strong);
    border-radius: var(--r-md);
    bottom: 16px;
    box-shadow: var(--shadow-popover);
    max-height: calc(100dvh - 32px);
    overflow-y: auto;
    position: fixed;
    right: 16px;
    width: 316px;
    z-index: 70;
  }
  @media (max-width: 520px) {
    .theme-dock { bottom: 8px; right: 8px; max-width: calc(100vw - 74px); }
  }
</style>
