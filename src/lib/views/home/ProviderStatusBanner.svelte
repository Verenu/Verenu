<script lang="ts">
  import type { ProviderStatusAlert } from '../../stores';
  import type { ProviderId } from '../../settings';
  import Callout from '../../components/Callout.svelte';
  import { getProviderLogo } from '../../setup/ProviderLogos';
  import { fly, fade } from 'svelte/transition';
  import { expoOut } from 'svelte/easing';
  import { MOTION_MS, MOTION_PX, motionMs, motionPx } from '../../motion';

  let { alerts }: { alerts: ProviderStatusAlert[] } = $props();

  function providerLogo(alert: ProviderStatusAlert): string | null {
    const id = alert.providerId.toLowerCase();
    if (!['groq', 'openai', 'google', 'assemblyai', 'openrouter', 'xai', 'local'].includes(id)) return null;
    return getProviderLogo(id as ProviderId);
  }

  async function openDetails(url: string) {
    if (!url.startsWith('https://') && !url.startsWith('http://')) {
      console.warn('Blocked opening non-HTTP(S) URL:', url);
      return;
    }
    try {
      const { open } = await import('@tauri-apps/plugin-shell');
      await open(url);
    } catch {
      window.open(url, '_blank');
    }
  }
</script>

{#if alerts.length > 0}
  <div
    class="status-banner"
    in:fly={{ y: -motionPx(MOTION_PX.nudge), duration: motionMs(MOTION_MS.panel), easing: expoOut }}
    out:fade={{ duration: motionMs(MOTION_MS.fast) }}
  >
    {#each alerts as alert, i (alert.providerId + '-' + i)}
      <Callout
        severity="warning"
        iconHtml={providerLogo(alert)}
        action={alert.detailsUrl ? { label: 'Check status', onclick: () => openDetails(alert.detailsUrl) } : undefined}
      ><strong>{alert.providerName}</strong> {alert.message}</Callout>
    {/each}
  </div>
{/if}

<style>
  .status-banner {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-bottom: 22px;
  }
</style>
